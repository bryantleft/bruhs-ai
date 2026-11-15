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

Closure revision (2026-11-15): branch `t3/close-calendar-pillar-items`, based
on `176d9d31`, verified at `16e5add3` (V1) and `bd3b7b87` (V2). It closes the
open items below on web and in source on desktop. Desktop GPUI tests written
for it compile and pass clippy but **have not run**: the V2 merge gate was
stopped by the disk floor while its test step was still compiling. Nothing
here was checked against a live Google account or in a real desktop window.

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
| CAL-01 | Switch Month, Week and Day; navigate Previous, Today, Next. The selected view is announced; targets meet the size standard on 1280×800, 640×480 and 390×844 touch. | **Verified on web** (V2 harness, all three cells). Desktop: GPUI frame test `calendar_views_draw_their_controls_at_the_parity_cell` compiles and lints clean, **not run**. Open on desktop. |
| CAL-02 | Create, edit and delete a local event, timed and all day. Reload restores it. | **Verified on web** (V2 harness, three cells): timed and all-day events created through the dialog, restored after a reload, deleted through the dialog and still absent after a reload; the all-day event spans local midnight to local midnight of the 25-hour day and does not spill into the next day. Desktop: unit and GPUI coverage compile, **not run**. Open on desktop. |
| CAL-03 | On the 2026-03-08 and 2026-11-01 Los Angeles DST days, a 10:00 event sits on the 10:00 row, clicking the 11:00 row prefills 11:00, and dragging to the 14:00 row saves 14:00 local. | **Verified on web** (V2 harness). Desktop draws the wall-clock view of stored instants; no GPUI DST run. |
| CAL-04 | Slot search (focus blocks, booking links) keeps working hours on both sides of a DST change inside its 30-day window. | **Verified in shared model tests** (V1: 66 passed). Desktop slot search and Today now read each event's instants, so events in the repeated fall-back hour are no longer dropped (`planning_keeps_events_in_the_repeated_fall_back_hour`, compiles, **not run**). |
| CAL-05 | A Google all-day date (`2026-11-02`) shows on that date only in every zone; previously stored records migrate once. | **Verified on web** (V2 harness, model tests). Desktop normalizes on ingest; no GPUI run. |
| CAL-06 | Google events are read only on every path: no drag, resize, edit, save or delete; the dialog offers the provider link instead. | **Verified on web** (V2 harness): forged drop refused, disabled fields and Save, provider link, no Delete, "Calendar color" instead of Persimmon for an uncolored event, Save in the neutral outline treatment with no persimmon fill. Desktop **implemented, unverified**: provider flag on `DesktopCalendarEvent`, no grab cursor, drag or resize edge, a read-only detail with the provider link, and a save guard; GPUI test `a_google_block_is_read_only_and_opens_its_provider_detail` compiles, **not run**. |
| CAL-07 | Sync only issues GET requests to the Calendar API with read-only scopes. | **Verified** with wiremock (`a_sync_only_reads_from_the_provider`, V1 core run) and scope source review. Live Google: awaiting external verification. |
| CAL-08 | A remote change to an untouched synced event applies directly; only an edit made after the last completed sync becomes a conflict. Accepting the remote version applies all of it. | **Verified in core and model tests** (V1). Accept remote now applies the remote title and notes as well as times (`accepting_remote_applies_the_remote_title_and_notes`, red before the fix in `target/calendar-qa/conflict-red.log`). Open: accepting stamps `updated_at` with the resolution time, so the first remote change in the next tick can still be deferred as a conflict. |
| CAL-09 | A failed calendar sync stays visible as an error and does not advance the last-sync time. | **Verified in core and model tests** (V1). Settings UI not driven. |
| CAL-10 | Resolving a conflict with Duplicate leaves an editable, deletable local copy without provider identity. | **Verified in model tests** (V1). |
| CAL-11 | Reconnect and offline: events stay readable offline; the stream reconnects with backoff and replays the window; replayed frames do not duplicate. | Partially verified: harness confirms a replayed frame does not duplicate. Offline/reconnect UI and live network loss are open. |
| CAL-12 | Recurrence: provider series instances appear across the visible range; local recurrence scope is decided. | Provider instances: **verified against the wiremock provider** (V1): the sync window is now `now − 7 days` to `now + 42 days` and is listed again once a day without breaking the incremental token chain; cursors from older builds list it once. RED/GREEN proof: with the relist disabled the three window tests failed on their intended assertions, and passed again on the byte-identical restored source. **Live Google unverified.** Events beyond 42 days ahead still arrive only when they change. Local recurring events: not implemented; open scope decision. |
| CAL-13 | Time zones: a change of the machine zone keeps instants on both surfaces. | Decision: desktop stores instants like the web. **Implemented, unverified on desktop**: the state file stores UTC instants plus the wall times older builds read; floating wall times migrate once in the current zone (a repeated fall-back hour as its first occurrence); loaded instants are kept while the wall time is unchanged. Migration, idempotence, zone-change and repeated-hour tests compile, **not run**. Limit: a zone change while the app runs takes effect at the next start. |
| CAL-14 | Task links: a due marker opens its task; a focus block opens its source task. | **Verified on web** (V2 harness, three cells): the due task, reached through the day list of a full day, and a focus block's Open task both open the task in Tasks with its title focused; an event without a source task offers no Open task. Desktop **implemented, unverified**: the due marker opens Tasks (it only selected the task before), the event dialog offers Open task, and the month day list reaches hidden records (`a_focus_block_opens_its_source_task`, `hidden_month_records_are_reachable_through_the_day_list`, compile, **not run**). |
| CAL-15 | Event dialog actions remain reachable at 640×480. | **Verified on web** (V2 harness). |
| CAL-16 | Keyboard: time-grid events are focusable and open with Enter or Space; controls have accessible names. | **Verified on web** for Enter on a focused block. Desktop blocks, day-list triggers and rows are focusable with Enter/Space handlers by source review; no GPUI keyboard run. |
| CAL-17 | Web and desktop parity with the same synthetic records at the same viewport. | Open: no real desktop window was drawn and no same-size desktop screenshot pass was run. |
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
| Accept remote applied only start and end, though conflicts are detected on title and notes too. | `SchedulingConflict::detected` records the remote title and notes; `apply_remote` applies them on web and desktop; older records keep local text. | Model test red, then green (V1). |
| Sync tokens only report changes, so instances of an unchanged recurring series never arrived after the first 15-day listing. | Daily relist of a `−7/+42` day window that only adds upserts ([sync_engine.rs](../bruhs-core/src/google_calendar/sync_engine.rs)). | Four wiremock tests; RED/GREEN proof in V1. |
| Month cells capped records at three and +N more was plain text, so a fourth record could not be opened from Month; on compact widths truncated pills shrank to zero height. Found by the V1 harness. | Cells clip and show three lines counting +N more; the day number and +N more open a "Calendar day" list of every event and due task, on web and desktop. | Web: V2 harness, hit-tested day number on all cells and +N more on 1280×800. Desktop: GPUI test compiles, not run. |
| Desktop `planning_calendar_events` used `.single()`, dropping events in the repeated fall-back hour from Today and slot search. | Planning reads each event's instants. | Unit test compiles, not run. |
| Desktop month due marker only selected the task. | Opens Tasks with the title focused, as the web does. | GPUI test updated, compiles, not run. |
| 90 `min-[hw]-N` classes in 24 web files generated no CSS. | Arbitrary pixel values with the same intent. | V2 built-CSS check: all 20 distinct classes have rules and no unsupported class remains. |

## Cross-pillar and open items

Closed in source on this branch (verification as in the case table): the
Today/day planning fall-back drop, the other `min-[hw]-N` users, the desktop
read-only affordance and inspection, desktop time semantics, Accept remote
text, the sync window, the Google dialog color and Save, and the focus block
back-link.

Still open:

- **Desktop verification:** every desktop GPUI and unit test added here
  compiles and passes clippy but has not run. Run the gate test step (it
  needs more free disk than the V2 window had) before calling any desktop
  item verified.
- **Live Google:** OAuth, sync window behavior, reconnect and offline.
- **Real desktop window and same-size screenshots** (CAL-17).
- **Desktop booking:** `#/book/<slug>` has no desktop surface.
- **Local recurring events:** not implemented; scope decision pending.
- **Accept remote stamping:** see CAL-08.
- **Large min-size classes on pages without fixtures:** the V2 screenshots
  rendered only the Life and Settings textareas (59→96px and 51→96px, below
  the fold, no visible change in the viewport). The Knowledge editor
  (320/384px), mail compose (160/192px), file preview (320px), whiteboard
  (64px) and rich media (48px) minimums were not exercised. No class was
  reverted.

## Evidence log

| Date | Cases | Work and verification | Remaining |
| --- | --- | --- | --- |
| 2026-11-14 | All | Inventory and cases recorded from source review of web, desktop, models, core and server. | Implementation and verification as listed. |
| 2026-11-14 | CAL-07 to CAL-10 | Red: `target/calendar-qa/core-red.log` (2 failing core tests). Green: see verification below. | Live provider acceptance. |
| 2026-11-14 | CAL-01, 03, 05, 06, 11, 15, 16 | Browser harness [`calendar_browser.mjs`](../bruhs-web/tests/calendar_browser.mjs) against a debug Trunk build, America/Los_Angeles, three cells. | Desktop GPUI checks, same-size parity screenshots. |
| 2026-11-15 | CAL-04, 07 to 10, 12 | V1 at `16e5add3`: fmt check; models tests (66 passed, 1 ignored); core `google_calendar` (66 passed); RED with the window relist disabled (0 passed, 3 failed on their intended assertions), source restored byte-identical, GREEN (3 passed); debug Trunk build; built-CSS check. The calendar harness then failed 168/6: a test bug (the Save check rejected the standard persimmon focus ring) and two product bugs (unreachable month records), fixed in the closure branch. | Live Google. |
| 2026-11-15 | CAL-01 to 03, 05, 06, 14 to 16 | Compile-only window at `bd3b7b87`: wasm `cargo check` and clippy for bruhs-web, all-targets `cargo check` and clippy for bruhs-desktop, all clean after three compile fixes. V2 at `bd3b7b87`: fmt check, debug Trunk build to `target/calendar-ui-dist-v2`, built-CSS check (20 classes, all with rules), calendar harness 236 passed 0 failed (80/78/78 across the three cells), inbox harness 399 passed 0 failed on the same bundle, min-size before/after screenshots (no revert needed). Inside the quick gate, fmt, workspace clippy (native, all targets and features) and wasm clippy passed; its test step was stopped at the 10 GiB disk floor while compiling, so no workspace tests ran. | Workspace tests, desktop GPUI tests, live Google, a real desktop window, same-size screenshots. |

### Verification

Audit runs (2026-11-14) are recorded in the audit branch. Closure runs, all
macOS (Darwin 25.3), debug profile, `CARGO_BUILD_JOBS=1`:

| Check | Command | Result |
| --- | --- | --- |
| Shared models | `cargo test -p bruhs-models --lib --locked -- calendar scheduling planning google_calendar` | V1: 66 passed, 1 ignored. |
| Core sync | `cargo test -p bruhs-core --lib --locked google_calendar` | V1: 66 passed. |
| Window relist RED/GREEN | the three window tests with `--exact`, relist disabled, then restored | V1: RED 0 passed 3 failed on the intended assertions; GREEN 3 passed on the restored bytes. |
| Compile | `cargo check` for bruhs-web (wasm32) and bruhs-desktop (all targets and features) | Clean at `bd3b7b87`. |
| Lints | `cargo clippy` for the same, `-D warnings`; workspace clippy native and wasm inside the V2 gate | Clean. |
| Web build | `trunk build --dist ../target/calendar-ui-dist-v2` | V2: built. |
| Built CSS | `target/calendar-qa/css-check.py` | V2: 20 classes, no missing rule, no unsupported class left. |
| Calendar browser | `node bruhs-web/tests/calendar_browser.mjs` | V2: 236 passed, 0 failed; 12 screenshots. |
| Inbox regression | `node bruhs-web/tests/inbox_browser.mjs` on the same bundle | V2: 399 passed, 0 failed. |
| Min sizes | `target/calendar-qa/min-size.mjs`, 1280×800 before/after | V2: reviewed, no revert. |
| Workspace tests | `scripts/gate.sh --quick` test step | **Not run**: stopped at the disk floor while compiling. |

Not verified: every desktop test added here (compiled and linted only), the
desktop app in a real window, the release bundle, live Google
OAuth/sync/reconnect, physical touch devices, screen readers, and
performance.

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
