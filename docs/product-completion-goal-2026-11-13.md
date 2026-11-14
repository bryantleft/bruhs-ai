# Product completion goal

Recorded 2026-11-13. **Status: active.** Bring Inbox and every other bruhs product
pillar to complete, usable functionality and full alignment with the bruhs design
system on web and desktop. This is accepted product scope, not a claim that the
work has shipped. Inbox is the first delivery priority.

“100% done” means every outcome below passes the shared acceptance gates with
recorded evidence. Existing code, passing unit tests, and a rendered page alone
do not establish completion. Keep unverified work open; do not invent progress
percentages or remove a difficult requirement to make the goal appear complete.

## Scope and ownership

Use the existing local-first architecture, shared domain models, native credential
storage, and per-domain agent permissions. Preserve intentional boundaries such
as read-only Google Calendar, manual finance/health records, reviewed external
writes, and explicit recording consent. Record any proposed expansion separately.

Web and desktop must expose equivalent supported outcomes and match visually at
the same viewport, with platform-appropriate interactions. Touch web and the
existing native mobile companion must support their documented flows; this goal
does not silently expand the companion into a copy of every desktop feature.

The [configurability goal](configurability-goal-2026-11-09.md),
[transcription goal](transcription-goal-2026-11-09.md), and
[performance goal](performance-goal-2026-11-09.md) remain the detailed plans for
those areas. Their unresolved criteria remain dependencies of this goal. Reuse
their evidence rather than creating competing settings, plans, or measurements.

Before implementation in each pillar, record a dated capability inventory for
each platform and convert every required outcome into numbered acceptance cases
with expected results. Include provider/device support and intentional exclusions.
Keep unimplemented requirements open. Record explicit scope changes rather than
silently redefining “supported” to exclude an unfinished flow.

## Acceptance gates for every pillar

- **Working outcomes:** discovery/setup, the primary action, inspection, editing,
  and recovery work through the real storage/provider path. A successful action
  has a durable result. Local logging, a proposed action, and remote execution
  have distinct labels and states. Unsupported capabilities are explicit.
- **Bruhs design:** use Monaspace typography, warm longan surfaces, semantic fruit
  colors, shared spacing/radius tokens, and tone/ring elevation. Reserve persimmon
  for brand actions and focus; use blueberry for selection/information. Keep
  primary content prominent and secondary controls quiet. Avoid drop shadows,
  gradients, decorative model meters, promotional banners, and dashboard cards
  that compete with the work. References may inform hierarchy, never replace
  bruhs's visual language.
- **Interaction and accessibility:** preserve keyboard-native navigation, visible
  focus, accessible names, stable hit areas, discoverable primary actions, and
  clear destructive-action recovery. Support reduced motion and text scaling.
  Follow the shared 13px UI, 12px small-label, 14px desktop-input scale and 44px
  touch targets with 16px touch inputs. Test 640×480 compact desktop, 390×844
  touch, and a larger desktop viewport per [UI standards](ui-standards.md).
- **Honest states:** distinguish initial setup, loading, no results, empty data,
  offline/stale data, permission denial, unavailable dependencies, failed saves,
  cancellation, and success. Errors retain recoverable input and offer a next
  step. Pending or failed provider operations cannot look completed.
- **Persistence and privacy:** verify reload/restart, migration, import/export,
  deletion/undo where supported, and failure recovery. Preserve domain grants,
  retention and opt-outs across agent context and derived records. Keep secrets
  out of portable state, messages, and logs. Local-first defaults stay intact.
- **Evidence and parity:** record the tested revision, fixtures, platform,
  viewport, steps, expected/actual outcomes, and evidence paths. Exercise the
  primary flow and meaningful failure cases on web and desktop with the same
  synthetic records. Compare same-size screenshots and keyboard/pointer flows;
  a screenshot alone does not prove behavior. Run focused behavioral checks and
  the repository's required gate before merging implementation changes. Measure
  performance through the actual user flow under the existing performance plan.

## Pillar completion criteria

All rows start **open** under this goal. “Audit required” means existing behavior
has not yet been checked against these gates, not that the pillar is unbuilt.

| Pillar | Outcomes required for acceptance | Starting status |
| --- | --- | --- |
| Inbox | Connect email, sync complete threads, read/search/filter, inspect sender/recipients/dates, compose/reply with attachments, manage drafts, archive/read/snooze, and recover from provider errors. Verify actual delivery and sync receipts. Initial email sequence is Gmail, then Outlook; record each provider's supported actions. Imported Slack/Discord/iMessage/X messages remain clearly identified as imports until real connectors pass acceptance. Preserve People links and agent assistance. | Partial local foundation; remote email and UX work open. |
| Today | Inspect schedule, ready work, overdue/waiting items, follow-ups, and recent decisions; open their source records; preview, apply, and undo a conflict-aware day plan. Handle empty days, overdue work, time zones, and changed source data. | Audit found stale Apply, Undo and DST failures; fixes remain open. |
| Chat and code | Choose runtime/model/access, start and resume threads, stream/cancel/retry, retain attachments and rich replies, inspect tools/diffs, use terminal/preview, and recover interrupted or failed turns without losing drafts/history. Import/export and project navigation preserve identity and permissions. | Audit required; reuse runtime and rich-reply evidence. |
| Workbench | Complete durable tasks, forks/delegation/comparisons, specifications and verification, worktrees, issue intake, reviewed delivery, usage inspection, and documented API/database/whiteboard tools. Verify cancellation, restart, missing tools, and paired/guest access boundaries. | Audit required; reuse Workbench and mobile validation. |
| Tasks and projects | Create/edit/organize/search tasks and projects; manage status, priority, dependencies, recurrence, cycles, estimates, and waiting-on context. Keep list/board/detail views consistent; scheduling and undo retain task identity and history. | Audit required. |
| Calendar | Navigate supported views, create/edit local events and focus blocks, resolve scheduling conflicts, and manage the supported external-calendar connection. Verify read-only Google sync, reconnect/offline behavior, recurrence, time zones, daylight-saving changes, and task links. | Audit in progress ([Calendar record](calendar-completion-2026-11-14.md)); read-only provider boundary remains. |
| Meetings | Set up supported devices/providers, record/stop/recover, follow live/final transcripts and speakers, inspect/edit notes and summaries, and reach linked tasks/people/chat. Pass the transcription and configurability goals with real device/provider limits documented. | Existing evidence is partial; goal acceptance open. |
| People | Create/find/edit profiles, follow source-backed observations and linked conversations/meetings, manage reminders/aliases/merges, undo supported changes, and honor per-person opt-outs across generation and retrieval. | Audit required. |
| Knowledge and capture | Capture supported record types, create/edit/find documents and clips, follow source links/backlinks/citations, inspect attachments, and export/delete/recover supported content. Verify global capture, search, offline use, and references after source edits or deletion. | Audit required. |
| Finance | Manage local accounts, transactions, budgets, income, holdings, and CSV import/export with correct amounts/currencies and duplicate/error review. Verify linked details, edits/deletion, and independent agent-access and exact-amount gates. | Audit required; manual ledger remains the scope. |
| Health | Manage workouts, sleep, recovery, and habits; inspect/edit histories and supported summaries. Verify dates/units, duplicate/error handling, persistence, and independent privacy gates for agent context and cross-domain insights. | Audit required; manual journal remains the scope. |
| Workflows and automations | Configure/review/enable, inspect activity and results, pause/resume/cancel, recover after restart, and handle failures without duplicate actions. Verify each supported trigger and per-domain grant, field-level previews, execution receipts, and undo/rollback where promised. | Audit required; reuse existing workflow evidence. |

Shared settings, global search, command navigation, secure service connections,
notifications, library/artifact access, and supported mobile/remote flows are part
of each affected pillar's acceptance. They cannot be left as an unowned final
polish task. The route inventory is
[`AppView`](../bruhs-models/src/models/ui.rs); include detail routes, search,
terminal, and booking when auditing their owning pillars.

## Inbox baseline and first work

The dated [Inbox capability inventory and numbered acceptance cases](inbox-completion-2026-11-13.md)
track implementation and verification for the first pillar.

This baseline is a source review from 2026-11-13. It is not a live visual,
accessibility, provider, or performance acceptance pass.

| Finding | Required change | Evidence |
| --- | --- | --- |
| Accounts are not connected; replies are logged locally. | Add real email account setup, sync, and delivery with capability and error states. Keep local imports/drafts usable offline. | [Web Inbox](../bruhs-web/src/components/inbox_page.rs), [shared send state](../bruhs-models/src/models/inbox.rs). |
| Summary metric cards precede messages; rows omit timestamps. | Put the conversation list first, reduce counts to compact navigation, and make sender/subject/preview/date easy to scan. | [Web Inbox](../bruhs-web/src/components/inbox_page.rs). |
| Persimmon marks unread, importance, and outgoing messages. | Apply the shared semantic-color rules; preserve unread and important meaning without relying on color alone. | [Web Inbox](../bruhs-web/src/components/inbox_page.rs), [native Inbox](../bruhs-desktop/src/views/workspace/inbox_ui.rs). |
| Native Inbox lacks the web search/filter controls and uses separate cards. | Match supported navigation, filtering, list hierarchy, and state visibility across both surfaces. Verify list/detail navigation and focus return. | [Native Inbox](../bruhs-desktop/src/views/workspace/inbox_ui.rs). |
| Native rows use 10–11px labels and a 6px radius outside the shared scale. | Adopt shared typography/control/radius tokens and verify density, text scaling, and touch behavior. | [Native Inbox](../bruhs-desktop/src/views/workspace/inbox_ui.rs), [UI standards](ui-standards.md). |

Choose list/detail behavior for wide and narrow windows by testing actual triage
and reply flows. Keep reading context and the next action easy to reach. The
provided email reference is inspiration for scanning hierarchy and consistent
rows; its light theme, model meter, upgrade banner, and styling are not a design
specification for bruhs.

## Delivery and tracking

Inbox is the first priority. The Gmail-then-Outlook implementation order and the
sequence of later pillars are provisional; adjust ordering for dependencies while
keeping the accepted outcomes and completion gates intact.

1. Finish the Inbox audit and capability matrix. Deliver hierarchy/token/parity
   fixes, then real email connection, sync, and delivery in reviewable slices.
2. Audit and complete Today, tasks/projects, and calendar as one planning flow.
3. Complete meetings, People, and knowledge/capture with source provenance and
   privacy checks across their links.
4. Complete finance and health independently, including their privacy boundaries.
5. Complete chat/code, Workbench, workflows, and automations, reusing existing
   implementation and validation. Address shared dependencies during every
   phase rather than waiting for this position in the sequence.
6. Run the cross-pillar regression and same-size web/desktop visual pass. Close
   the goal only when every pillar and dependency has passing acceptance evidence.

For each delivery, record the pillar, concrete acceptance case, status,
implementation revision/files, verification command or steps, evidence path,
and remaining dependency. Use **open**, **in progress**, **awaiting external
verification**, or **verified**. Failed cases return to open; a provider/device
check waiting on access stays awaiting external verification. Local tests and
transport fakes cannot satisfy live provider or physical-device acceptance.

| Date | Area | Progress | Remaining |
| --- | --- | --- | --- |
| 2026-11-13 | Goal and baseline | Defined all pillar outcomes, shared gates, and source-backed Inbox gaps. | Implementation, remaining pillar audits, and acceptance checks stay open. |
| 2026-11-13 | Inbox implementation | Added dated capability/acceptance cases and the first hierarchy/search/filter parity changes. Shared Inbox tests pass; browser harness prepared. | Current shared builds are blocked by concurrent Whiteboard wiring. Native/browser acceptance, live email, and other pillar work remain open; see the [Inbox evidence log](inbox-completion-2026-11-13.md). |
| 2026-11-13 | Inbox verification and mail grants | Coordinated compile checkpoints resolved the prior build blockers. All 4 native Inbox and 8 shared Inbox checks pass. Initial production browser run passed 384 automated checks but manual review found compact search/history failures. Fixes and stronger assertions are written. Separate Gmail/Outlook authorization passes 18 synthetic connection tests and actual WASM check. | Recheck compact layouts from the combined gate's fresh bundle. Live mailbox sync, sending, attachment flows, and full Inbox acceptance remain open. Rust sources are temporarily frozen for the coordinated format/gate, with the Git index owned by the finalization thread. |
| 2026-11-13 | Chat/code unification evidence | The owning thread supplied `target/unified-threads-qa/validation.json` and `report.json`; inspected results show 63 actual-WASM browser checks across desktop/compact/touch and focused native/web tests passing for unified navigation and project attachment. | This verifies that subset only. The broader chat/code outcomes and final current-tree gate remain open; reuse this evidence during that pillar's audit. |
| 2026-11-14 | Default release web verification | Immutable default capture `gate-dist-capture-20261114T104636Z` passes Inbox 399/399 and synthetic mail setup 642/642. All eight prior touch failures are resolved; asset/source/artifact hashes and visual samples were independently checked. The shared workspace gate passes 3,010 tests, strict native/WASM lint, release checks, rustdoc and dependency policy. | No live provider login, sync, send or full Inbox completion. |
| 2026-11-14 | Today release audit | The unchanged harness reports 404 passing checks and 63 failing assertions: stale Apply 45, Undo/newer-edit loss 12, DST workday arithmetic 6. Assertion sets match the prior debug run with no regressions; these are three failure families, not 63 distinct bugs or muted expected failures. Evidence is retained under `target/inbox-ui-qa/release-checkpoint-20261114T104636Z/`. | Behavior fixes remain off-tree. Rollover/resume, delayed/failed persistence, native parity/performance and live providers remain unverified. |
| 2026-11-14 | Integrated Today fixes | Shared Apply/Undo ownership validation, explicit local workday bounds, transactional web receipts, queued-edit preservation and mounted-clock refresh are integrated. The new pinned production bundle passes 935 Today assertions plus the unchanged Inbox 399 and synthetic mail 642 checks. All original 63 failures are resolved without removing assertions. Native focused controls/timezone checks and strict native/WASM lint pass. See the [case-by-case Today record](today-completion-2026-11-14.md). | Full Today states, keyboard/scaling/motion, native physical/restart/failure parity and performance remain open. Live mail and the wider product goal remain open. The finalization source hold is released; the separate no-index/history-write agreement remains in force. |
| 2026-11-14 | Calendar audit | Inventory and 18 numbered cases recorded. Fixed false sync conflicts, hidden sync failures, DST time-grid and slot-search errors, Google all-day date shifts, editable Google events on web, read-only conflict copies, and header/dialog accessibility. Model/core tests failed first and now pass; a new Los Angeles DST browser harness passes on three cells. See the [Calendar record](calendar-completion-2026-11-14.md). | Desktop read-only affordance and inspection, time-semantics parity, recurring-series sync window, focus-block back-links, live Google sync and same-size parity screenshots remain open. |

Goal registration creates no scheduled job, connects no account, changes no
privacy grant, and sends no email. Use synthetic data for automated checks;
live external actions retain their normal explicit product authorization.
