# People completion checklist

Recorded 2026-11-14. Parent: [product completion goal](product-completion-goal-2026-11-13.md).
**Status: in progress.** This inventory records the People capability baseline,
fixed acceptance cases, and the first privacy and integrity fixes. It does not
certify completion. The parent goal's shared gates still apply.

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
| PEOPLE-01 | Create a profile from the People page on each surface, with Unicode and long names. Blank names are refused and input is kept. Reload restores it. | **Failing.** No People-page create on either surface; Capture-only path unverified here. |
| PEOPLE-02 | Find a person on the People page by name and alias, case-insensitively, including beyond the first page. | **Failing.** No People-page search on either surface. |
| PEOPLE-03 | Rename with trimming. A blank name is refused with a visible message and sends nothing. A failed save is visible. Reload keeps the new name. | **Verified on web** (browser harness, three cells). Desktop trims and refuses blanks by source review; desktop failure copy changed, not exercised. |
| PEOPLE-04 | Add, remove and inspect aliases. Aliases match speakers and senders case-insensitively, including non-ASCII names. | **Failing** for editing (no UI). Matching **verified** in core for summaries, appended observations and recognized speaker names. |
| PEOPLE-05 | Each observation shows its source meeting and opens it. An observation whose meeting is gone says so. No observation is dropped from the list. | **Verified** on web (browser harness) and desktop (GPUI test-frame check). |
| PEOPLE-06 | Open a person's meetings and conversations from the profile and return. | **Failing.** Only unanswered-conversation suggestions exist; desktop suggestions are not clickable. |
| PEOPLE-07 | Create, edit, complete and delete reminders with the same options on both surfaces. State survives reload. | **Failing.** No delete; desktop lacks the optional due date and kind choice. Persistence covered by existing store tests only. |
| PEOPLE-08 | Merge after confirmation. Aliases, observations, reminders, voice and all links move to the target. An opt-out on either side survives. | **Failing.** No confirmation; web does not repoint Inbox or knowledge links (cross-pillar). Sticky opt-out **verified** by existing core tests (`merge_retry_tests`, `sqlite/tests.rs`). |
| PEOPLE-09 | Undo a merge, rename, opt-out change and deletion; both records come back intact. | **Failing.** Only workflow reminder undo exists; merge originals are archived but never restored. |
| PEOPLE-10 | Delete a profile with confirmation or undo; observations, aliases and links follow a documented cascade. | **Failing.** Not supported. |
| PEOPLE-11 | With a person opted out, generating or regenerating a summary, appending observations, or recognizing their name or alias on screen adds no observation for them under any casing. Reopening a database written by an older build keeps the protection. | **Verified** in core: 8 new tests in `meetings::opt_out_tests` on both stores, plus existing `summary_tests` tracking cases. |
| PEOPLE-12 | With a person opted out, no agent read returns their observations; identity and the opt-out stay visible to the agent. | **Verified** on desktop (GPUI bridge test). Web has no agent People reads. |
| PEOPLE-13 | Toggling the opt-out shows a visible failure when the save fails and does not look applied. A confirmed change survives reload. | **Verified on web** (browser harness). Desktop shows a notice by source review, not exercised. |
| PEOPLE-14 | Deleting a meeting removes or explicitly retains its derived observations by a documented rule. | **Failing.** Observations remain with a dangling source (`delete_observations_for_meeting` is unused outside tests). The UI now says “Source meeting is not stored”. Needs a product decision. |
| PEOPLE-15 | Compare web and desktop People pages with the same records at the same viewport. | **Unverified.** No same-size desktop screenshot was taken. Known gaps listed in the inventory. |
| PEOPLE-16 | Keyboard and screen reader: every card action has a unique accessible name, focus is visible, and touch targets are 44px. | **Partially verified on web** (names and touch targets for the checked controls, harness). The web rename input still sits inside the expand button. Desktop keyboard activation of text buttons is unverified. |
| PEOPLE-17 | 640×480, 390×844 touch and 1280×800 without horizontal overflow or hidden actions. | **Verified on web** for the checked flows (harness). Desktop unverified. |
| PEOPLE-18 | Honest states: loading, empty, cache failure, sync failure, merge recovery, failed save. | **Partial.** Web has them; desktop lacks loading and merge recovery UI. Not exercised beyond PEOPLE-03 and PEOPLE-13. |
| PEOPLE-19 | Measure People list and profile performance with a large directory. | **Unverified.** Existing server read-performance tests only. |

## Cross-pillar items

These need files owned by other pillars. They were not edited.

- **Inbox (desktop auto-link).** [`inbox_ui.rs`](../bruhs-desktop/src/views/workspace/inbox_ui.rs)
  links an imported sender with `display_name.eq_ignore_ascii_case`. That ignores
  aliases and non-ASCII case, and takes the first duplicate. It should use
  `bruhs_models::person_names_match` across display names and aliases, and refuse
  ambiguous matches.
- **Inbox (web merge links).** The web merge commit
  ([`commit.rs`](../bruhs-web/src/storage/people/merge/commit.rs)) does not
  repoint Inbox participants or knowledge links to the surviving contact. The
  desktop does. The fix writes to the Inbox store, so it needs coordination with
  the Inbox owner.
- **Meetings.** Deleting a meeting leaves its observations behind (PEOPLE-14).

## Evidence log

| Date | Cases | Work and verification | Remaining |
| --- | --- | --- | --- |
| 2026-11-14 | All | Source inventory and fixed acceptance cases recorded. | Implementation and verification of failing cases. |
| 2026-11-14 | PEOPLE-04, PEOPLE-11 | Red first: 5 of the new core opt-out tests failed (an opted-out `Zoë` let a new tracked `ZOË` profile collect the observation on both stores; legacy index rows missed), then 2 named-speaker tests failed (an alias minted a new profile). Fixed with one Unicode case-folded name key, alias-aware speaker recognition, and a rekey of legacy sqlite name rows on open (no schema version change). `cargo test -p bruhs-core --lib meetings::`: 122 passed, 1 ignored. `cargo test -p bruhs-models --lib person`: 10 passed. | Alias editing UI. |
| 2026-11-14 | PEOPLE-12 | Red first: the desktop bridge test failed. Fixed in `people_read.rs`; MCP tool descriptions updated. `cargo test -p bruhs-desktop --bin bruhs-desktop people_read`: 8 then 9 passed. | Web has no agent People reads. |
| 2026-11-14 | PEOPLE-03, PEOPLE-05, PEOPLE-13, PEOPLE-16, PEOPLE-17 | Web and desktop UI fixes: visible rename and opt-out failures, blank-name refusal, opt-out explanation, accessible names, 44px touch sizing on card actions, observation source links, plain copy. Desktop GPUI check for the explanation and source navigation passes. First browser harness run on a debug bundle (`target/people-ui-qa-run1/`): 56 passed, 9 failed. All 9 failures were the source-link checks, 3 per cell: the web hydrates meetings only on the Meetings page, so a cold People visit labeled every source as not stored. Fixed by hydrating meetings from People and withholding the missing-source text until loading finishes. | Rerun of the harness and wasm check after the shared measurement hold. |
