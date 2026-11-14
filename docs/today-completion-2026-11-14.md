# Today completion evidence

Recorded 2026-11-14. Parent: [product completion goal](product-completion-goal-2026-11-13.md).
**Status: in progress.** The four audited planning defects are fixed in the
working tree. The full Today and product acceptance gates remain open.

## Implementation and platform inventory

The shared planner accepts explicit local 09:00–18:00 workday bounds. Apply
revalidates current task eligibility, dependency state, event identity and
calendar conflicts. Undo tracks ownership of generated events and task links;
it preserves later task edits and refuses to silently delete a modified or
independently adopted generated block.

Web commits tasks, events and the Undo receipt in one real IndexedDB transaction.
The app owns that transaction across navigation; edits made while it is pending
queue behind it and retain their newer fields. Failed commits leave the previous
durable state intact. The receipt survives reload. Today refreshes its clock on
a timer, focus, pageshow and visibility changes, invalidating an old day's review.

Desktop uses the same validation and ownership rules, includes the receipt in
the persisted workspace snapshot, and uses the existing save/recovery coordinator.
Pending saves are labeled as saving. Its existing background timer refreshes the
mounted Today view. Focused GPUI controls and snapshot serialization are tested;
physical-window, accessibility and failure/restart acceptance remain open.

Implementation: [shared planning](../bruhs-models/src/models/planning/day_plan.rs),
[web transaction](../bruhs-web/src/storage/day_plans.rs),
[app-owned writer](../bruhs-web/src/task_saves/planning.rs),
[web Today](../bruhs-web/src/components/today_page.rs), and
[desktop Today](../bruhs-desktop/src/views/workspace/today_ui.rs).

## Fixed acceptance cases and remaining scope

These IDs retain the scenarios defined in the original source audit. A verified
subset does not close untested parts of the same case.

| Case | Required outcome | Current evidence and remaining work |
| --- | --- | --- |
| TODAY-01 | Review, Apply and Undo atomically; reload preserves results and supported Undo. | Verified on actual web WASM/IndexedDB. Focused native controls and receipt serialization pass; native process restart remains open. |
| TODAY-02 | A newly conflicting event invalidates a reviewed proposal. | Verified through real durable web changes and focused native stale-source checks. |
| TODAY-03 | Deleted, completed, blocked or independently scheduled tasks cannot create stale/orphan blocks. | Web deletion/completion and native deletion/completion/conflict pass. Shared tests cover eligibility/dependency/link validation; remaining combinations need surface acceptance. |
| TODAY-04 | Replayed Apply or an existing generated ID cannot duplicate blocks/links. | Shared validation rejects duplicate/colliding IDs. Explicit surface replay acceptance remains open. |
| TODAY-05 | Undo preserves later title/status/description and planning changes. | Verified on web, including reload and actual local edits during pending commits; focused native preservation check passes. |
| TODAY-06 | Undo preserves or explicitly refuses a modified/adopted generated block. | Shared ownership tests pass. Direct web/native editor acceptance remains open. |
| TODAY-07 | Delayed/failed Apply or Undo retains newer edits and truthful recovery. | Verified web native transactions: quota failure/rollback/reload/retry, pending commits and explicit aborts with newer Tasks edits. Native save-failure/restart and physical crash/power loss remain open. |
| TODAY-08 | LA spring/fall transitions, normal dates and another zone retain 09:00–18:00 local hours. | Actual web DST/adjacent-day/UTC checks pass. Native Local-zone tests run in isolated LA/UTC child processes; physical timezone changes remain open. |
| TODAY-09 | Empty/full days, overdue/waiting work and missing sources have usable copy/navigation. | Full state and source-navigation audit remains open. |
| TODAY-10 | Desktop/compact/touch, keyboard, reduced motion and text scaling meet the shared gates. | Web 1280×800, 640×480 and 390×844 touch planning flows pass with visual review. Full keyboard, scaling, motion, same-size native parity and performance remain open. |
| TODAY-11 | Mounted rollover/resume refreshes date, agenda bounds and review validity. | Web controlled time advance with real focus/visibility events passes in all three viewports. Natural OS suspension, background scheduling, timezone switching and native mounted acceptance remain open. |

## Versioned verification

The accepted base `cdf35b428d39a2ff0c369c246574820b8e29867f` previously
reported 404 passing and 63 failing Today assertions across three families:
stale Apply (45), Undo/newer edits (12), and DST (6). That immutable evidence is
preserved; it is not relabeled as passing.

The integrated default production capture is
`target/today-integrated-ui-dist-20261114T123807Z`:
298 assets, API `http://127.0.0.1:3001`, default features, `NO_COLOR=true`.
Its index SHA256 is
`88462254aafcc4a71d747749239fa6f5435f458f066003965cb535d51bfdd888`.
`target/pillar-completion-qa/today-integration/build.json` records every asset;
`build-inputs.json` records the source capture. Later core/server mail work and
other owners' stream work are outside that pinned WASM acceptance.

| Browser suite | Assertions | Result |
| --- | ---: | --- |
| Original 23 Today scenarios | 467 | 0 failed; every original assertion identity retained and all 63 prior failures resolved. |
| Mounted clock, three viewports | 63 | 0 failed. |
| Apply/Undo quota recovery, six scenarios | 135 | 0 failed; genuine native IndexedDB abort/rollback. |
| Pending/aborted commits with newer UI edits, 12 scenarios | 270 | 0 failed; six commits and six aborts, route/reload/receipt preservation. |
| Existing Inbox regression | 399 | 0 failed. |
| Existing synthetic mail setup | 642 | 0 failed; synthetic transport only. |

These are **1,976 assertions with no failures**, not 1,976 distinct bugs or
complete product acceptance. Reports and hash manifests are under
`target/pillar-completion-qa/today-integration/browser-20261114T125949Z/` and
`concurrency-20261114T131926Z/`. They retain 181 screenshots. Review covered 53
critical frames from the first five runs and all 36 concurrency frames; it does
not claim that every retained screenshot was directly reviewed. No unexpected
runtime/provider requests or external writes occurred. The mail setup suite's
three deliberate Gmail-start 503 console records are identified in its report.

Compiled verification includes 25 focused shared planning checks, the full
model/web suites (1,007 passing, six ignored), native Apply/Undo controls and
isolated native timezone checks. Strict model/web/native lint and actual-WASM
checks passed; the latest native control edit was separately rechecked with
strict native and WASM Clippy. Logs are in
`target/pillar-completion-qa/today-integration/`. The earlier full workspace gate
belongs to the committed base; no new whole-workspace gate is implied here.

## Reproduction

Build only when a current matching capture is unavailable. Supply its explicit
path; the scripts refuse stale runtime sources and use an existing Playwright
installation (`BRUHS_PLAYWRIGHT_PATH` can select one). All records are synthetic.
Historical local audit JSONs are optional report context.

```sh
node bruhs-web/tests/today_browser.mjs /absolute/path/to/fresh/dist
node bruhs-web/tests/today_clock_browser.mjs /absolute/path/to/fresh/dist
node bruhs-web/tests/today_failure_browser.mjs /absolute/path/to/fresh/dist
node bruhs-web/tests/today_concurrency_browser.mjs /absolute/path/to/fresh/dist
cargo test -p bruhs-models --lib models::planning --locked --offline
cargo test -p bruhs-web task_saves --locked --offline
cargo test -p bruhs-desktop desktop_today_plan_review_apply_and_undo_should_be_one_transaction --locked --offline
cargo test -p bruhs-desktop native_workday_preserves_local_hours_on_dst_transition_dates --locked --offline
```

The controlled browser clock, quota exception, pending-commit read pump and
explicit transaction abort are stated test inputs. They exercise the real app
and native storage, and do not establish physical crash, live provider, natural
OS resume, native/device, or performance acceptance.
