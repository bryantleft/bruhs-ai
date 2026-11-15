# Inbox completion checklist

Recorded 2026-11-13. Parent: [product completion goal](product-completion-goal-2026-11-13.md).
**Status: in progress.** This fixes the initial Inbox scope and acceptance cases;
it does not certify completion. Reuse the parent goal's shared gates.

## Capability inventory

| Capability | Web baseline | Desktop baseline | Required outcome |
| --- | --- | --- | --- |
| Local conversation import | Paste sender, subject, message, and source. | Same local import with inline form. | Keep imports usable offline; identify imported source honestly. |
| Search and filtering | Full indexed text search and five filters. | Inbox-only list with pagination. | Search sender/address/subject/full message and expose Inbox, Unread, Snoozed, Archived, All on both surfaces. |
| Reading and triage | List/detail, mark read/unread, archive, snooze, importance. | Same actions; different list layout. | Reach every stored conversation, expose dates and state, preserve full content and focus. |
| Drafts and replies | Save draft and log reply locally. | Same local behavior. | Persist local drafts; distinguish provider send acceptance from actual recipient delivery. |
| People and agent context | Link person and explicitly ask agent. | Auto-link matching imported sender; explicitly ask agent. | Equivalent explicit linking and bounded, untrusted agent context on both surfaces. |
| Delete/recovery | Immediate local deletion. | Reviewed local deletion. | Consistent confirmation or undo, durable recovery behavior, no accidental remote deletion. |
| Account connection and sync | No email connector. | No email connector. | Gmail and Outlook account setup, reconnect/disconnect, thread sync, and visible capability/error state. |
| Compose and attachments | No remote compose/attachment flow. | No remote compose/attachment flow. | New email, threaded replies, attachments, save/resume, safe retry, and actual delivery. |
| Non-email providers | Source labels for pasted imports. | Same source labels. | Slack, Discord, iMessage, and X remain labeled imports. Live non-email connectors are excluded from this initial email milestone. |
| Performance and responsive UI | Bounded paged projection; prior partial measurements. | Bounded paged rendering; prior partial measurements. | Preserve full-text reachability and measured performance while fixing compact/touch layouts and parity. |

The milestone includes real Gmail and Outlook email. Imported conversations and
local reply logging remain supported alongside them. Source-label availability
is not evidence of a live connector. Company support/privacy email forwarding is
separate from the product Inbox. Later non-email connectors require an explicit
scope addition; they are not a substitute for completing email.

Provider send acceptance and recipient delivery are distinct. Record the provider
message/thread identity when a send is accepted; claim delivery only when the
provider supplies delivery evidence. Live acceptance uses an explicitly authorized
test recipient to verify the actual message. Reconcile interrupted sends against
the provider's sent records before offering a retry that could duplicate a message.
If a provider cannot confirm delivery, show send acceptance and unknown delivery
separately; verify the actual test message in the authorized recipient mailbox.
Attachment search covers filename and metadata; arbitrary attachment-body indexing
is not included in this milestone.

## Numbered acceptance cases

All cases begin open. Promote a case only for the platforms and paths actually
tested; preserve unverified visual, device, and live-provider checks separately.

| Case | Scenario and expected result | Initial status |
| --- | --- | --- |
| INBOX-01 | Import a message on each surface, including Unicode and long content. Missing sender/body shows an error and retains input. Reload restores the full conversation. | Open. |
| INBOX-02 | Search sender, participant address, subject, and text beyond the truncated preview, case-insensitively. Changing search resets/clamps pagination; clearing it restores records. | Open. |
| INBOX-03 | Exercise all five filters and paginate through more than one page. Archived, unread, and snoozed records remain reachable. Triage changes update membership and compact counts immediately. | Open. |
| INBOX-04 | Open/read, mark unread, archive/unarchive, star/unstar, snooze/unsnooze, and return to the list. State survives reload; snooze expiration updates the visible list without a manual reload. | Open. |
| INBOX-05 | Rows show sender, subject, preview, source, date, and unread/important meaning with restrained hierarchy. Saving a draft does not change the displayed received/sent date. Unknown/invalid dates stay explicit. | Open. |
| INBOX-06 | Save a reply draft, navigate away, reopen, restart/reload, and continue it. Failed persistence retains the draft and reports the failure. | Open. |
| INBOX-07 | Log a local reply and verify its durable local history. The interface never claims that a logged reply was delivered remotely. | Open. |
| INBOX-08 | Link/unlink a person explicitly and open the linked record. Ask agent inserts bounded reference content without sending or granting extra access. Imported message instructions remain untrusted. | Open. |
| INBOX-09 | Delete a conversation through confirmation or undo, cancel/recover it, reload, and verify all supported recovery guarantees. Local deletion never implies provider deletion. | Open. |
| INBOX-10 | Compare web/desktop with identical records and viewport. Navigation, search/filter results, counts, row hierarchy, dates, semantic colors, typography, and primary actions agree. | Open. |
| INBOX-11 | Complete import, triage, search, draft, and navigation using the keyboard. Focus is visible and returns meaningfully after dialogs/detail navigation. Controls have accessible names and state. | Open. |
| INBOX-12 | Exercise 640×480, 390×844 touch web, and larger desktop with long names/subjects, text scaling, and reduced motion. No lost controls, clipped content, hidden actions, or undersized touch targets. | Open. |
| INBOX-13 | Distinguish first import, empty Inbox, no search results, empty filtered view, loading, and storage/provider failure. Each message explains the state and offers the appropriate next step without losing work. | Open. |
| INBOX-14 | Verify per-domain agent grants and data isolation, account disconnect/revocation, exported non-secret data, and absence of credentials in logs/browser state. | Open. |
| INBOX-15 | Connect Gmail, sync complete threads and participants, search/read attachments, and recover from reconnect, expired credentials, incremental-sync gaps, and offline use without duplicate messages. | Open; live-provider acceptance required. |
| INBOX-16 | Compose/send/reply with attachments through Gmail, verify recipient/subject/thread and provider send acceptance, verify receipt at an authorized test mailbox, and safely reconcile interrupted or ambiguous sends. | Open; live-provider acceptance required. |
| INBOX-17 | Repeat connection/sync/send/reply/attachment acceptance with Outlook and explicitly represent provider capability differences. | Open; live-provider acceptance required. |
| INBOX-18 | Restart or disconnect during sync/send and resume correctly. Concurrent changes preserve newer drafts and records; duplicate/replayed events do not duplicate delivery or discard edits. | Open. |
| INBOX-19 | Measure search/navigation/input-to-paint with representative small/large histories, startup/offline/impaired links, and native rendering per the performance goal. Preserve complete histories and bounded mounting. | Open; existing evidence is partial. |
| INBOX-20 | Export and reimport supported conversations/drafts/attachments with provenance and identity; handle malformed/conflicting imports without partial silent success. | Open. |

## Email connection dependency

The native service broker now has separate Gmail and Outlook Mail authorization
entries alongside the existing Google/Microsoft Calendar entries. The latter's
read-only scopes are unchanged. Registration, token storage/refresh, and explicit
mail-scope validation are implemented. Gmail read/sync is integrated; delivery
and live-provider acceptance remain open.
The server now explicitly opens and owns the connected-mail SQLite cache and
reviewed-send journal. Its async API retains current broker leases, serializes
blocking writes, preserves newer draft revisions and marks interrupted sends
uncertain on exclusive restart. Local imported records remain independent.
This storage/runtime foundation alone does not establish real account, send,
attachment or delivery acceptance.
The [runtime contract and evidence](mail-runtime.md) describes its ownership,
revision and interrupted-send guarantees.
The [Gmail adapter](gmail-sync.md) now has controlled-transport read/full/history
sync tests. The [connected Email UI](connected-mail-ui-2026-11-14.md) now uses real
server transport on web/native; its fresh production browser acceptance and live
account acceptance remain open.
The implementation is in
[`connections/oauth.rs`](../bruhs-core/src/connections/oauth.rs) and the
[service connection guide](service-connections.md). Live registrations, OS keychain
prompts, provider policy, and actual mail operation acceptance remain open.

## Evidence log

Record implementation files/revision, exact checks, synthetic fixtures, platform,
viewport, evidence paths, and remaining limitations for each case. A local or fake
transport pass does not close live delivery. Keep the goal active until all cases
and the parent gates pass.

| Date | Cases | Work and verification | Remaining |
| --- | --- | --- | --- |
| 2026-11-13 | All | Source inventory and fixed acceptance cases recorded. | Implementation and verification remain open. |
| 2026-11-13 | INBOX-02, INBOX-03, INBOX-05, INBOX-10, INBOX-12, INBOX-13 | Implemented native indexed search and five filters, retained web browse state, message dates, compact counts below the list, semantic colors, shared native sizing, and clearer empty/import states. Added long-text wrapping and touch sizing to changed web controls. Shared Inbox tests: 8 passed. | Browser execution and current native regression rerun remain unverified; full-pillar acceptance remains open. |
| 2026-11-13 | INBOX-02, INBOX-03, INBOX-05, INBOX-12 | All 4 native Inbox checks and 8 shared Inbox checks pass at the coordinated compile checkpoint. First production browser run passed 384 automated checks with no runtime errors or external writes. Visual review found a 20px-wide compact search field and roughly 30px of message history; fixes and stronger geometry assertions are written. | Fresh browser execution and visual review of the compact fixes remain pending. Prior screenshots/results are preserved under `target/inbox-ui-qa/before-compact-fix`. |
| 2026-11-13 | INBOX-14, INBOX-15, INBOX-17 | Separate Gmail/Outlook authorization entries and mail-scope enforcement added. All 18 connection tests pass, including partial-grant rejection, rotated-token preservation after narrowed permissions, Calendar isolation, and mail-secret removal from agent commands. | No live provider login or mailbox sync/send acceptance. Core evidence is `target/inbox-ui-qa/mail-connections.log`; this is authorization groundwork, not a completed email client. |
| 2026-11-14 | Local web Inbox and synthetic connection UI | The captured default production bundle passes all 399 Inbox and 642 synthetic mail-setup checks. All 30 Inbox screenshots were reviewed; compact search/history fixes pass. Five Permissions and three Today Undo touch failures are resolved with 44px targets. Asset, source and artifact hashes were independently checked. The shared workspace gate passes 3,010 tests plus strict lint/build/docs/dependency checks. | Full Inbox/native/device/provider acceptance remains open. No live login, sync, send or delivery was exercised. |
| 2026-11-14 | Connected Email reading, sync and attachment UI | The later corrective default capture passes 1,525 browser checks across desktop, compact and touch, with all 36 synthetic downloads verified by bytes, hashes and names. Feedback remains visible during reader scroll. Controlled runtime/native checkpoints pass 84 core mail, four production-router HTTP and five native Inbox tests. See [versioned evidence and limits](connected-mail-ui-2026-11-14.md). | This is not live-provider, physical-native/device, compose/send or recipient-delivery acceptance. Earlier Inbox/Today/setup results retain their own bundle scope; the current full-workspace gate and pillar remain open. |

The shared model check is reproducible with
`cargo test -p bruhs-models --lib models::inbox --locked`. At the recorded implementation
checkpoints, changed Rust files passed targeted `rustfmt --check` and the browser
harness passed `node --check`. The finalization gate passed at
`cdf35b428d39a2ff0c369c246574820b8e29867f`; the later Today/mail working changes
have separately scoped checks and do not inherit a new whole-workspace gate.
Generated evidence is in `target/inbox-ui-qa/models.log` and
`target/inbox-ui-qa/implementation.json`, including source hashes.

The native regression drives search by keyboard, checks archived/full-text
reachability, and checks index refresh after an edit with unchanged record count
and timestamp. All four native Inbox checks passed at the captured checkpoint, including pagination and
IPC draft persistence/undo. These are GPUI test-frame checks, not physical-window
or complete accessibility acceptance. Source hashes and limits are recorded in
`target/inbox-ui-qa/native-evidence.json`.

The first production build and its browser run completed after coordinated source
checkpoints. Its source/artifact hashes are in `target/inbox-ui-qa/build-source.json`.
The final default production bundle was captured from `target/gate/dist` into
`target/gate-dist-capture-20261114T104636Z`, preserving all 298 asset hashes and
timestamps. The 27 relevant source files and three unchanged harnesses match the
pinned manifest. Use `BRUHS_INBOX_UI_DIST` to reuse that capture rather than
starting a duplicate build during finalization. The current web subset passes;
the native/policy gate also passed for that committed base; broader product
cases and the newer implementation remain separate.
For an independent reproduction after the source/build window is released:

```sh
cargo test -p bruhs-desktop --bin bruhs-desktop inbox_ --locked
cd bruhs-web
env NO_COLOR=true trunk build --release --dist ../target/inbox-ui-dist
cd ..
node bruhs-web/tests/inbox_browser.mjs
```

The [browser harness](../bruhs-web/tests/inbox_browser.mjs) uses isolated browser
contexts and the real IndexedDB writer at 1280×800, 640×480, and 390×844 touch.
It is designed to check imports/validation, triage, saved drafts/local replies and reload,
dates, full-text search, pagination, filter retention, overflow, and touch targets.
It refuses stale assets and unexpected external writes. Its first execution
passed 384 automated checks, but manual review found compact search/history
failures that those checks missed. The assertions now require full-row search,
computed reading space, and visible initial message text. The release run passes
399 checks with visual review of all 30 Inbox frames. Local evidence is preserved
under `target/inbox-ui-qa/release-checkpoint-20261114T104636Z/`; its verification
report also records the unchanged open Today failures. No full Inbox acceptance
or live email delivery is claimed.

### Combined validation follow-up, 2026-11-15 UTC

The [combined checkpoint](mail-finalization-2026-11-15.md) records the later
14-step gate and focused copy acceptance. It supersedes pending final-gate/copy
status for that source snapshot; earlier evidence retains its original scope.
Recovery, Sent reconciliation and live-provider/product acceptance remain open.
