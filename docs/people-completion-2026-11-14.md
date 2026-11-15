# People completion checklist

Recorded 2026-11-14. Parent: [product completion goal](product-completion-goal-2026-11-13.md).
**Status: in progress (PARTIAL).** This inventory records the People capability
baseline, fixed acceptance cases, the decided rules, and the fixes made against
them. As of 2026-11-15 the branch passes the workspace **quick** gate (G3,
see the evidence log); that is not the full gate and does not certify
completion. Open items are listed at the end. The parent goal's shared gates
still apply.

Statuses use the parent's vocabulary. **Verified** means the case passed the
checks named in the evidence column on the platforms listed there, and nothing
more. **Failing** means the requirement is missing or a check failed.
**Unverified** means the behavior may exist but was not exercised.

## Opt-out semantics

The per-person opt-out is `PersonProfile.tracked == false`. On both surfaces it
is labeled Track or Untrack, and the profile shows “Not tracked”. From this
audit onward, the opt-out means:

1. **Generation.** Summaries add no observations about the person, matched by
   linked speaker, display name or alias. Names are compared case-insensitively
   with Unicode case folding (`bruhs_models::person_name_key`). Regenerating a
   meeting keeps the person's existing observations from that meeting. The
   global setting and the per-meeting `skip_observations` flag still apply on
   top of this.
2. **Retrieval.** Agents never receive the person's stored observations. The
   desktop bridge's `bruhs_read_person` observations section returns an empty
   page with `observations_withheld: true`. `bruhs_read_people` reports a null
   `observation_count` and `observations_withheld: true`. Identity (ID, name,
   aliases, tracked flag) stays discoverable so an agent can see and respect
   the opt-out.
3. **Unchanged.** The user still sees their own saved observations on the
   People page and in global search. User-authored relationship notes and
   reminders are not covered by observation tracking. Meeting transcripts and
   summaries still contain what the person said. The opt-out covers derived
   person observations, not meeting content. Excluding a person from
   transcripts or summaries entirely would be a new scope item; it is not
   claimed here.

4. **Merge and undo.** A merge keeps the opt-out sticky: the survivor is
   untracked if either side was. Undoing a merge gives each contact back its
   own flag, unless the survivor's flag was changed after the merge, in which
   case that later choice is kept.

Agent context paths reviewed for opted-out data:

| Path | Finding | Status |
| --- | --- | --- |
| Summary prompt ([`summarize/prompt.rs`](../bruhs-core/src/summarize/prompt.rs)) | Holds only the transcript. No stored observations or profiles. | Verified by source review |
| Summary commit ([`summary_update.rs`](../bruhs-core/src/meetings/summary_update.rs), [`contacts.rs`](../bruhs-core/src/meetings/sqlite/contacts.rs), [`in_memory.rs`](../bruhs-core/src/meetings/in_memory.rs)) | Dropped observations for untracked people, but ASCII-only name matching let `ZOË` miss an opted-out `Zoë` and mint a new tracked profile that collected them. | **Fixed.** Red then green, see PEOPLE-11 |
| Recognized speaker names ([`speaker_linking.rs`](../bruhs-core/src/meetings/speaker_linking.rs)) | Matched display names only. A name merged into an opted-out contact minted a new tracked profile. | **Fixed.** Red then green, see PEOPLE-11 |
| Desktop agent bridge ([`people_read.rs`](../bruhs-desktop/src/views/workspace/mcp_bridge/requests/people_read.rs)) | `bruhs_read_person` returned an opted-out person's full observation history. | **Fixed.** Red then green, see PEOPLE-12 |
| Meeting chat seed ([`store.rs`](../bruhs-core/src/store.rs) `create_thread_for_meeting`) | Meeting metadata and transcript only. | Verified by source review |
| Inbox “Ask agent” ([`inbox.rs`](../bruhs-models/src/models/inbox.rs) `inbox_agent_prompt`) | Conversation content only, no profile data. | Verified by source review |
| Web agent tools | The web hosts no People read bridge. Agents started from the web cannot read People records. | Not applicable; recorded as a parity gap |
| Summary route response ([`meetings_summary.rs`](../bruhs-server/src/routes/meetings_summary.rs)) | Returns the summary only, not raw model observations. | Verified by source review |

## Decided rules

These were product decisions for this pillar. Both surfaces and both stores
apply them.

- **Delete cascade (PEOPLE-10).** Deleting a profile asks for confirmation
  first. The contact's observations, aliases and private relationship data
  (details and reminders) are deleted. Meeting speakers, summary action
  items, Inbox participants and knowledge links stay where they are and lose
  only their link to the person. Merge redirects that ended at the contact
  are retired with it. Capture receipts are left unchanged. The server keeps
  a private record of what it removed so the delete can be undone.
- **Undo (PEOPLE-09).** Rename, the opt-out toggle, merge and delete offer
  undo right after the change, from an undo bar on the People page. Undo
  follows the workflow reminder rule: it reverts a change only while the
  records it touched are exactly as the change left them, and otherwise
  refuses with "No records were changed." The receipt lives in memory for
  the session: a reload or a newer People change replaces it. Undoing a delete
  restores the contact, its relationship data, and the speaker, action item,
  Inbox and knowledge links it removed that are still unlinked. Undoing a
  merge restores both contacts from the merge originals, keeps later edits
  to the survivor, and points back the links the merge moved. Merges made by
  builds before this one restore the contacts but not their meeting links,
  because those builds did not record them.
- **Meeting delete (PEOPLE-14).** Deleting a meeting deletes the observations
  derived from it in the same store commit, with the same behavior on the
  in-memory and SQLite stores. The server announces `PeopleUpdated` only when
  the delete actually removed derived observations, so both surfaces refetch
  then. Deleting a missing meeting, a failed delete, or deleting a meeting
  with no derived observations announces nothing. No surface offers a meeting
  delete control yet; the rule applies to the server route and any future
  control.
- **Alias uniqueness (PEOPLE-04).** A new alias is refused when another
  profile already answers to that name (display name or alias, Unicode case
  folded). The refusal says to merge the two profiles instead. The surface
  reviews it before sending and the server reviews it again before saving.
- **Inbox auto link.** An imported sender links to a person only when exactly
  one profile answers to the sender's name or alias. Two matches link
  nobody.

## Capability inventory

| Capability | Web baseline | Desktop baseline | Required outcome |
| --- | --- | --- | --- |
| Create profile | Only through the Capture dialog's contact kind. No create action on the People page. | Same, through Capture. | A discoverable create flow on the People page on both surfaces. |
| Find | No People page search. Pages of 25. Global search indexes names, aliases and observation text. | No People page search, no pagination. Command palette indexes names and aliases. | Search by name and alias on the People page; consistent paging. |
| Rename | Enter or blur commits. Before this audit: no trim, blank names sent, failures only logged. | Trims, ignores blank, failure notice. | Validated rename with visible failure on both. |
| Aliases | No add or remove UI. Aliases only grow through merges. | Same. | Add, remove and inspect aliases; uniqueness review. |
| Observations | Grouped by kind. Before this audit: rows sharing a meeting and timestamp collided on one list key, and no source link. | Grouped by kind, no source link. | Every observation shown with a link to its source meeting, or an explicit missing-source state. |
| Linked meetings and conversations | Unanswered Inbox conversations only, as clickable follow-up suggestions. No list of a person's meetings. | Suggestions as plain text, not clickable. | A person's meetings and conversations reachable from the profile on both surfaces. |
| Reminders and relationship items | Four kinds, optional due date, toggle complete. No delete. | One cycling kind, due tomorrow forced for two kinds, toggle complete. No delete. | Create, edit, complete and delete with equivalent options. |
| Merge | Journaled merge with recovery UI. No confirmation step, no undo. Inbox and knowledge links are not repointed. | Merge with archives; repoints Inbox, knowledge and capture links. No confirmation, no undo, no recovery UI. | Confirmed merge, recoverable undo, all links repointed on both. |
| Undo | Workflow-created reminders only. | Same. | Undo for supported People changes (merge, rename, opt-out, delete). |
| Delete profile | Not supported. | Not supported. | Delete with confirmation or undo and a defined cascade. |
| Opt-out | Toggle with a visible state. Before this audit: no explanation, failures only logged, visible label showed state rather than action. | Toggle with state badge and per-person accessible name. No explanation. | Explained opt-out, honest failure states, enforced in generation and retrieval. |
| Agent access | No People read tool on the web. | `bruhs_read_people` and `bruhs_read_person` behind the People Read grant. | Grant-gated, opt-out-respecting reads; web parity or documented exclusion. |

## Numbered acceptance cases

| Case | Scenario and expected result | Status |
| --- | --- | --- |
| PEOPLE-01 | Create a profile from the People page on each surface, with Unicode and long names. Blank names are refused and input is kept. Reload restores it. | **Verified, scoped.** Web: r2 browser harness (historical, see the evidence log). Desktop: GPUI test frames, not a real window. The desktop `people_page_create_refuses_blank_names_and_keeps_unicode_names` test passes in G3. |
| PEOPLE-02 | Find a person on the People page by name and alias, case-insensitively, including beyond the first page. | **Verified, scoped.** Web: r2 browser harness (historical, see the evidence log). The harness reaches an alias on page two. Desktop: GPUI test frames, not a real window. Desktop `people_search_finds_an_alias_beyond_the_first_page` passes after the pager test fix (D1, G3). |
| PEOPLE-03 | Rename with trimming. A blank name is refused with a visible message and sends nothing. A failed save is visible. Reload keeps the new name. | **Verified on web** (browser harness, three cells). Desktop trims and refuses blanks by source review; desktop failure copy changed, not exercised. |
| PEOPLE-04 | Add, remove and inspect aliases. Aliases match speakers and senders case-insensitively, including non-ASCII names. | **Verified, scoped** for editing. Web: r2 browser harness (historical, see the evidence log). Desktop: GPUI test frames, not a real window. Desktop `alias_that_another_profile_uses_is_refused_and_kept` passes in G3. The server refusal is covered by core and route tests in G3. Matching **verified** in core for summaries, appended observations and recognized speaker names. |
| PEOPLE-05 | Each observation shows its source meeting and opens it. An observation whose meeting is gone says so. No observation is dropped from the list. | **Verified** on web (browser harness) and desktop (GPUI test-frame check). |
| PEOPLE-06 | Open a person's meetings and conversations from the profile and return. | **Verified on web only, scoped.** Web: r2 browser harness (historical, see the evidence log). The harness opens a linked meeting and returns with Back to People. Desktop has no test for the linked records or the way back, and desktop follow-up suggestions are only checked to compile (G3). |
| PEOPLE-07 | Create, edit, complete and delete reminders with the same options on both surfaces. State survives reload. | **Verified, scoped.** Web: r2 browser harness (historical, see the evidence log). The harness covers add, edit, complete and delete, and reload. Desktop: GPUI test frames, not a real window. Desktop `reminders_use_kind_and_due_options_and_can_be_edited_and_deleted` passes in G3. |
| PEOPLE-08 | Merge after confirmation. Aliases, observations, reminders, voice and all links move to the target. An opt-out on either side survives. | **Verified, scoped.** Web: r2 browser harness (historical, see the evidence log). The harness checks that nothing merges before confirmation. Desktop: GPUI test frames, not a real window. Desktop `merge_sends_nothing_until_confirmed` passes in G3. The web commit's repointing of Inbox participants and knowledge links compiles (G3) but is not exercised by a browser check. Sticky opt-out **verified** by core tests. |
| PEOPLE-09 | Undo a merge, rename, opt-out change and deletion; both records come back intact. | **Partial.** Web: r2 browser harness (historical, see the evidence log). The harness undoes a merge and a delete. Desktop: GPUI test frames, not a real window. Desktop delete then undo passes in G3. Server restore and unmerge on both stores and the `PeopleUndo` rules pass in G3; the undo summary punctuation fix passes in M1 and G3. Not exercised in any UI: the rename and opt-out undo round trips, and a desktop merge undo. |
| PEOPLE-10 | Delete a profile with confirmation or undo; observations, aliases and links follow a documented cascade. | **Verified, scoped.** Web: r2 browser harness (historical, see the evidence log). Desktop: GPUI test frames, not a real window. Desktop `delete_needs_confirmation_cascades_and_undo_restores` passes in G3. The store cascade passes on both stores in G3. |
| PEOPLE-11 | With a person opted out, generating or regenerating a summary, appending observations, or recognizing their name or alias on screen adds no observation for them under any casing. Reopening a database written by an older build keeps the protection. | **Verified** in core: 8 new tests in `meetings::opt_out_tests` on both stores, plus existing `summary_tests` tracking cases. |
| PEOPLE-12 | With a person opted out, no agent read returns their observations; identity and the opt-out stay visible to the agent. | **Verified** on desktop (GPUI bridge test). Web has no agent People reads. |
| PEOPLE-13 | Toggling the opt-out shows a visible failure when the save fails and does not look applied. A confirmed change survives reload. | **Verified on web** (browser harness). Desktop shows a notice by source review, not exercised. |
| PEOPLE-14 | Deleting a meeting removes or explicitly retains its derived observations by a documented rule. | **Verified in core and server.** The rule (see Decided rules) passes on both stores, and the route announces `PeopleUpdated` exactly once only when observations were removed (S2, G3). The unchanged meeting-write performance contract passes again. No surface offers a meeting delete control, so there is no UI check. |
| PEOPLE-15 | Compare web and desktop People pages with the same records at the same viewport. | **Unverified.** No same-size desktop screenshot was taken. Known gaps listed in the inventory. |
| PEOPLE-16 | Keyboard and screen reader: every card action has a unique accessible name, focus is visible, and touch targets are 44px. | **Partially verified.** Web: r2 browser harness (historical, see the evidence log). Names and touch targets for the checked controls; the web rename field sits beside the actions, outside the expand button. Desktop: GPUI test frames, not a real window. Desktop `people_text_buttons_activate_from_the_keyboard` passes in G3. No full keyboard or screen reader pass on either surface. |
| PEOPLE-17 | 640×480, 390×844 touch and 1280×800 without horizontal overflow or hidden actions. | **Verified on web** for the checked flows (harness, 2026-11-14 and the historical r2 run). Desktop native window unverified. |
| PEOPLE-18 | Honest states: loading, empty, cache failure, sync failure, merge recovery, failed save. | **Partial.** Web has them. Desktop shows a loading state until the first restore or refresh settles and a merge recovery section with retry; its wording is close to the web's but the desktop keeps different recovery data and has no recovery data viewer. Desktop code compiles and the quick gate passes (G3), but no check exercises these states. |
| PEOPLE-19 | Measure People list and profile performance with a large directory. | **Unverified: diagnostic only.** P3 ran 5,000 profiles × 3 runs on an instrumented debug bundle (see the evidence log). No performance criterion exists and none is claimed. |

## Cross-pillar items

These touch files owned by other pillars. The edits are kept small and are
listed so a landing can merge them with care.

- **Inbox (desktop auto link).** [`inbox_ui.rs`](../bruhs-desktop/src/views/workspace/inbox_ui.rs)
  used `display_name.eq_ignore_ascii_case` and took the first duplicate. It
  now uses `bruhs_models::unique_person_named`: display names and aliases,
  Unicode case folded, and no link when two profiles match. The desktop
  `inbox_import_links_by_alias_and_refuses_ambiguous_names` test passes in G3.
- **Inbox (web merge links).** The web merge commit
  ([`commit.rs`](../bruhs-web/src/storage/people/merge/commit.rs)) now
  repoints Inbox participants and knowledge links to the survivor in the
  same IndexedDB transaction and records them for undo. It compiles (G3 wasm
  clippy) but no browser check exercises it.
- **Inbox and meetings (Back to People).** The conversation and meeting
  detail headers show a Back to People button only when opened from a
  profile ([`inbox_page.rs`](../bruhs-web/src/components/inbox_page.rs),
  [`meeting_detail.rs`](../bruhs-web/src/components/meeting_detail.rs)).
- **Meetings (PEOPLE-14).** `delete_meeting` on both stores deletes derived
  observations in the same commit; the route announces `PeopleUpdated` only
  when some were removed (S2, G3).

## Evidence log

| Date | Cases | Work and verification | Remaining |
| --- | --- | --- | --- |
| 2026-11-14 | All | Source inventory and fixed acceptance cases recorded. | Implementation and verification of failing cases. |
| 2026-11-14 | PEOPLE-04, PEOPLE-11 | Red first: 5 of the new core opt-out tests failed (an opted-out `Zoë` let a new tracked `ZOË` profile collect the observation on both stores; legacy index rows missed), then 2 named-speaker tests failed (an alias minted a new profile). Fixed with one Unicode case-folded name key, alias-aware speaker recognition, and a rekey of legacy sqlite name rows on open (no schema version change). `cargo test -p bruhs-core --lib meetings::`: 122 passed, 1 ignored. `cargo test -p bruhs-models --lib person`: 10 passed. | Alias editing UI. |
| 2026-11-14 | PEOPLE-12 | Red first: the desktop bridge test failed. Fixed in `people_read.rs`; MCP tool descriptions updated. `cargo test -p bruhs-desktop --bin bruhs-desktop people_read`: 8 then 9 passed. | Web has no agent People reads. |
| 2026-11-14 | PEOPLE-03, PEOPLE-05, PEOPLE-13, PEOPLE-16, PEOPLE-17 | Web and desktop UI fixes: visible rename and opt-out failures, blank-name refusal, opt-out explanation, accessible names, 44px touch sizing on card actions, observation source links, plain copy. Desktop GPUI check for the explanation and source navigation passes. First browser harness run on a debug bundle (`target/people-ui-qa-run1/`): 56 passed, 9 failed. All 9 failures were the source-link checks, 3 per cell: the web hydrates meetings only on the Meetings page, so a cold People visit labeled every source as not stored. Fixed by hydrating meetings from People and withholding the missing-source text until loading finishes. | Superseded by the next row. |
| 2026-11-14 | PEOPLE-03, PEOPLE-05, PEOPLE-13, PEOPLE-16, PEOPLE-17 | Meeting hydration fix rechecked: `cargo clippy -p bruhs-web --target wasm32-unknown-unknown -- -D warnings` passes. Harness rerun: 111 of 111 passed. Visual review of the compact frame found the opt-out badge and count wrapping, then (after a first fix) card actions overflowing the card by about 19px. Neither was caught by the assertions, so a containment assertion was added. It failed red on compact and touch (4 failures), then passed after the header was made to wrap. Final run on a debug bundle: **117 passed, 0 failed** across 1280×800, 640×480 and 390×844 touch, 15 screenshots reviewed in `target/people-ui-qa/`. | Release bundle not built. Desktop same-size comparison not done. |
| 2026-11-14 | Gate | `scripts/gate.sh --quick` was started and stopped during its first (fmt) step because free disk was 19 GB, below the agreed 20 GB floor for this shared machine. **The workspace gate has not passed on this branch.** Checks that did run: targeted tests above, `cargo check` of bruhs-desktop, wasm32 clippy of bruhs-web, `cargo fmt --all`. | Run `scripts/gate.sh --quick` (and the full gate) when disk allows. |
| 2026-11-14 | PEOPLE-01, 02, 04, 06, 07, 08, 09, 10, 14, 16, 18, 19, cross-pillar | Source only, under a machine-wide build hold: nothing below was compiled, formatted or run. Shared rules in `bruhs_models` (`people_directory`, `people_changes`, `people_copy`, reminder edit and remove) with unit tests. Core: `delete_person`, `restore_person`, `unmerge_person`, `set_person_aliases` and the meeting delete cascade on both stores, sqlite schema 6, 15 new store tests in `meetings::people_lifecycle_tests` written against both stores. Server: four People routes and `PeopleUpdated` on meeting delete, with route tests. Web: create, search, aliases, linked records, reminders, confirmations, delete, undo, merge link repointing; People browser harness extended (create, search, aliases, linked records, reminders, merge and delete undo) and a 5,000-person timing run added. Desktop: the same flows in a new People view, 8 GPUI tests, Inbox auto link fix. | Compile, fix, run every queued check below, then move cases to Verified only on that evidence. Same-size desktop screenshots (PEOPLE-15, PEOPLE-17). |
| 2026-11-15 | All Rust | **P1, compile only** (run `p1-iterative-20261115T115322Z`): fmt, check of models, core, server, web wasm, web tests and desktop, web wasm clippy and People clippy, all nine clean on one HEAD with zero warnings. No tests ran. | Superseded by the runs below. |
| 2026-11-15 | PEOPLE-01, 02, 04, 06, 07, 08, 09, 10, 16, 17 | **r2 People browser harness, HISTORICAL** (run `p2-r2-run-20261115T125531Z`, debug bundle `target/people-ui-dist-r2`): 292 passed, 0 failed across 1280×800, 640×480 and 390×844 touch, 36 screenshots. That bundle predates the undo copy change (d24d5ec4), so it is not evidence for the current undo summary text. | Rerun on a current bundle before landing claims beyond this. |
| 2026-11-15 | Inbox, PEOPLE-19 | **P3** (run `p3-run-20261115T134516Z`): Inbox browser harness 399 passed, 0 failed on the r2 bundle. Scale: 5,000 profiles × 3 runs, debug bundle, instrumented driver, diagnostic only; medians list 578 ms, search 1315 ms, profile 85 ms; page one was in order in all 3 runs. Not performance acceptance. The first scale attempt (r2) timed out because the fixture served all 5,000 versions in one page, over the 256 page limit the web enforces; the fixture was fixed to page like the server in 986fca92. | Performance acceptance needs a criterion and a release bundle. |
| 2026-11-15 | Seams | Four seams found by the runs and fixed: the legacy `merged_people` column fixture named its columns (test fix, 43123d4e); the desktop pager click landed below the 1080 px test viewport, confirmed by a fixture diagnostic, and the test now scrolls to the pager (test fix, 89d794d8); undo summaries doubled a final period (product copy fix, d24d5ec4); `PeopleUpdated` was announced on every meeting delete and is now announced only when observations were removed (product fix, 087b9518). | None for these seams. |
| 2026-11-15 | PEOPLE-09 | **M1** (run `people-m1-run-20261115T154156Z`): focused undo copy tests 6/0, default and serial, plus strict native and wasm lints. | |
| 2026-11-15 | PEOPLE-02 | **D1** (run `pager-d1-run-20261115T145755Z`): diagnostic confirmed the Next centre outside the test viewport; with the fix, the People page test module 7/0, default and serial, plus strict desktop clippy. | |
| 2026-11-15 | PEOPLE-14 | **S2** (run `people-s2-run-20261115T161912Z`): core meetings 139/0/1, server meeting routes 12/0 (including no event without observations and on a failed delete), unchanged meeting-write performance contract 1/0, each default and serial, plus strict lints. | |
| 2026-11-15 | Gate | **G3 quick gate** (run `20261115T162756Z`, HEAD 087b9518): unmodified `scripts/gate.sh --quick`, all 9 steps (fmt, clippy, clippy-wasm, test, web-assets, site-test, site-types, mobile, bench-check) passed exactly once; 32 suites, 3446 passed, 0 failed, 54 ignored. This is the quick gate only, not the full gate. | Full gate. Open items below. |

## Open items

Known and still open after G3:

- A profile restored by undo is not brought into view; it joins the end of
  the directory order.
- The merge and delete confirmations can read "Delete Ada L.?" for a name
  that ends in a period.
- No real desktop window was drawn and no same-size web and desktop
  screenshots exist (PEOPLE-15, PEOPLE-17 native).
- No full accessibility or screen reader pass (PEOPLE-16).
- Live providers and the full product were not exercised.
- No performance acceptance (PEOPLE-19 is diagnostic only).
- Undo receipts are lost on reload.
- Merges made by builds before schema 6 undo without their meeting links.
- No delete meeting control on either surface.
- Builds older than schema 6 refuse a database this branch has opened.

Reproduce:

```sh
cargo test -p bruhs-models --lib person --locked
cargo test -p bruhs-models --lib people_ --locked
cargo test -p bruhs-models --lib relationship --locked
cargo test -p bruhs-core --lib meetings:: --locked
cargo test -p bruhs-server people --locked
cargo test -p bruhs-server meetings --locked
cargo test -p bruhs-desktop --bin bruhs-desktop people --locked
cargo test -p bruhs-desktop --bin bruhs-desktop inbox --locked
(cd bruhs-web && trunk build --dist ../target/people-ui-dist)
node bruhs-web/tests/people_browser.mjs
BRUHS_PEOPLE_SCALE=5000 node bruhs-web/tests/people_scale_browser.mjs
```

The harness uses isolated browser contexts and the real IndexedDB cache. It
answers People, meeting and status reads with synthetic loopback fixtures,
fails on any other write or any non-loopback request, and refuses a bundle
older than the People sources. It is a web check only: no desktop window,
live provider or physical touch device was exercised.
