# Compose, replies and reviewed sending

Status on 2026-11-14: common draft/MIME storage, both provider transports and
compose HTTP routes are implemented. Web and native editors are being verified.
The current combined UI has no acceptance claim yet. No live account was used
and no real email was sent for this work. Provider acceptance is separate from
recipient delivery. The product completion goal remains open.

## Drafts and files

Editable To/Cc/Bcc, subject and plain-text body are stored separately from imported
Inbox conversations. Incomplete recipient text remains savable. Review parses
quoted display names, comma/semicolon lists and international text, validates
all recipients, retains To/Cc/Bcc placement and rejects header injection. Writable
requests cannot choose From, MIME bytes, send receipts or a replacement reply
source. Reply and Reply all are distinct actions; received Bcc is never inferred.

Server drafts use revision checks, immutable reply targets and indexed, bounded
list metadata. Conflicts retain the newer server revision. Response-lost retries
cannot reset an existing draft. A reply is bound to the selected canonical cached
message and provider thread, including its exact subject and checked RFC reply
headers. Current broker access supplies the verified SMTP sender at review and
again before dispatch.

A selected file belongs to one mailbox, draft and client-generated upload id.
Its basename, size, SHA256 and binary bytes are immutable. Reusing an id with
changed bytes or metadata fails. Bruhs allows 20 files and 16 MiB combined,
including empty files; these are application limits. Review additionally applies
the selected provider's MIME limit. The server checks selected bytes and digests.
Files cannot be removed while referenced or needed by a pending/uncertain send.
A typed accepted message retains its frozen MIME; explicitly unlinked orphan
uploads may then be removed without changing the reviewed message.

Schema 4 adds outgoing attachments. Schema 5 adds immutable prepared MIME blobs
and indexed draft metadata, preserving existing host identity, mail, cursors,
drafts and send operations. Preparation and its draft revision commit together.
The prepared reference binds raw bytes, review metadata and editable content.
Edits clear preparation. A retry/restart reuses the same frozen bytes; corruption
fails closed. Provider access is required for preparation and dispatch, while
local editing and draft files remain available after disconnect.

## Device persistence

The browser keeps scoped draft copies and native Blob bytes in the existing
IndexedDB metadata store. Text, newly selected files and the thin list index
commit in one revision-checked transaction. A request success alone never
publishes Saved. App-owned ordered writers retain newer edits during saves and
route changes. A failed transaction keeps input and file handles available for
explicit retry. Separate tabs cannot overwrite a newer device revision.

Native uses a separate private SQLite device store, keyed by server origin,
server identity and mailbox. It holds no credentials or provider capability.
Text and newly selected bytes commit together with compare-and-save, immutable
file checks, thin indexed pages and explicit process ownership. Its blocking
worker retains that ownership when a caller disappears. Graceful shutdown saves
outstanding text and already captured file bytes; abrupt termination before a device commit remains a limit.
Unsent work stays independent of the imported Inbox/workspace snapshot.

Both editors expose New email, Drafts, Reply and Reply all, file selection/removal,
an explicit full review and Send. Review shows verified From, To/Cc/Bcc, subject,
plain-text body and selected files. Pending sends keep the reviewed draft read
only. The operation id is saved on device before the send request. Restoring a
pending operation reads saved status and never automatically posts Send again.

## HTTP and dispatch

The compose router adds these paths under `/api/mail/mailboxes/{mailbox}`:

| Path | Methods | Behavior |
| --- | --- | --- |
| `/drafts` | GET, POST | Bounded revision-fenced list or idempotent create |
| `/drafts/{client}` | GET, PUT | Read or compare-and-save editable content |
| `/drafts/{client}/files/{file}` | POST, DELETE | One bounded multipart file or explicit unlinked removal |
| `/drafts/{client}/review` | POST | Current-account verification and immutable MIME preparation |
| `/drafts/{client}/send` | POST | Reviewed revision/digest and durable operation id |
| `/sends/{operation}` | GET | Host/mailbox-scoped persisted status |
| `/sends/{operation}/cancel` | POST | Cancel only an operation that is still queued |

Every request checks server identity and canonical mailbox provenance. Provider
operations resolve the owning provider and current connection lease. Existing
imported/Gmail/Outlook read, sync, download, origin policy and safe page-boundary
sync Stop are preserved. JSON/upload bodies are bounded and writable fields are
closed to injected sender, state and MIME properties.

The runtime owns the lease and outgoing task beyond the HTTP/UI waiter. Only an
atomic durable claim constructs the non-Clone, non-serializable dispatch token.
Adapters consume it and execute one fixed-origin POST with redirects and retries
disabled. Explicit known-unaccepted responses are rejected. Ambiguous responses,
timeouts, redirects and interrupted dispatch become Uncertain, with no automatic
replay. Acknowledgement persistence failure also remains uncertain. An accepted
review consumes only its exact revision, retaining a newer server edit.

Gmail sends the exact frozen MIME as base64url JSON, with a checked provider
thread id for replies. Outlook sends standard-base64 MIME as text/plain and
accepts 202 without inventing message ids. Its 3,000,000 raw-byte preflight keeps
the outer base64 request within 4,000,000 bytes. Outlook Mail requires delegated
User.Read for its actual SMTP profile lookup as well as Mail.ReadWrite/Mail.Send.
The separate [Outlook checkpoint](evidence/outlook-reviewed-send-2026-11-14.json)
and [consent documentation](service-connections.md) describe its scoped proof.

## Verification and remaining acceptance

Historical receipts remain versioned to their captured sources:

- `target/inbox-ui-qa/mail-compose-integration/backend-checkpoint.json`: 135 Core
  mail tests, nine existing mail router tests, strict Core/Server lint.
- `target/inbox-ui-qa/mail-compose-integration/dispatch-checkpoint.json`: 147 Core
  mail tests and the controlled one-attempt worker contract.
- `target/inbox-ui-qa/mail-compose-integration/provider-send-checkpoint.json`:
  165 Core mail tests, 10 model tests, nine existing router tests and strict lint,
  including actual synthetic HTTP requests for both provider adapters. Its former
  User.Read finding is resolved by the later Outlook consent receipt, without
  retroactively changing that historical scope.
- `target/outlook-qa/outlook-consent-checkpoint.json`: 20 connections tests,
  165 Core mail tests and strict Core lint against the consent repair.

The current backend capture on the integrated base passes 14 mail model tests,
20 connection tests, 170 Core mail tests and all 14 mail-router HTTP tests,
plus strict Core/Server all-target/all-feature lint. All 900 scoped inputs remain
unchanged. Receipt:
`target/inbox-ui-qa/mail-compose-integration/review-backend-checkpoint.json`;
source manifest SHA256
`2a743469f64344d7ed1b6c672342ef20953bf6ea6f5df14786f9c59fb1333861`.

The five new actual-router tests use the real native HTTP client for disconnected
editing/CAS, immutable uploads and frozen Gmail sending, Outlook 202 optional ids,
request/host/revision fences and a disconnected caller whose single provider POST
still persists its acknowledgement. Five device-store tests cover atomic
text/files, rollback, restart, scope isolation, integrity, matching file-id
preservation during an online refresh and an atomic shutdown write against an
older queued writer. These focused results do not establish complete UI behavior.

The earlier native compose checkpoint passes three GPUI tests with 1,451 fixed
source pins at `current-check-20261114T223722Z`: real input and chooser bytes remain
durable off-route after server failure, a restored pending send issues only a
status GET, and the shutdown path retains the last staged input/file bytes. An
independent review subsequently found seven recovery issues. Their repairs are
applied; the two device-store repairs pass the current backend capture, while
production browser verification of the complete repair set remains pending.
Seven current native composer tests subsequently pass in the shared desktop run,
including reserved file selection and exact lost-save recovery. Its only suite
failure was a missing composer focus style. The focused repair passes both
accessibility audits and strict desktop lint. The current web suite passes
465 tests with five ignored, plus strict host/actual-WASM lint; all 1,473 captured
source hashes stay fixed. Receipts are
`target/inbox-ui-qa/mail-compose-integration/native-owner-reuse-20261115T0000Z/receipt.json`
and `target/inbox-ui-qa/mail-compose-integration/current-ui-compile-checkpoint.json`.
The immutable review and exact patch are retained under
`target/inbox-ui-qa/mail-compose-review-20261114T2244Z/`.

The subsequent default production capture passes 1,066 compose checks, 358
warm-app Slow 3G checks, and the unchanged strict read driver in Gmail mode
(1,595) and Outlook mode (1,771). Screenshots cover 320–1,920px, compact/touch
controls and complete review details. Injected storage/upload/save/send failures
are matched to exact requests; uncertain sends survive reload without replay.
These are synthetic transport checks against real WASM/IndexedDB/Blob/FormData.

Physical macOS checks then exposed a short message field, a Command-Enter footer
collision, and missing accessibility text. Both new behavioral regressions fail
before the repairs; all nine native composer tests pass afterward, as do both
accessibility audits, strict Core/Server/Desktop lint and a normal app build.
The corrected app passes real OS file selection, exact Unicode/275-byte storage,
keyboard review/send, accessible status/review metadata, and restart recovery at
1,280×800 and 640×480. One synthetic send POST remains one after restart and two
status GETs. Input value/focus exposure and full VoiceOver acceptance remain open.

Exact evidence and version limits are recorded in
[the UI checkpoint](evidence/mail-compose-ui-2026-11-14.json). After acceptance,
only two copy changes were added: one-byte plural handling on web and a reactive
warning range in shared models. They await the combined gate's fresh capture;
the accepted bundle is retained unchanged.

Remaining work includes current production browser/native acceptance, failure
and restart coverage across the complete flows, provider Sent reconciliation for
ambiguous attempts, and concrete live-account consent/send review. Sent lookup
must correlate the Message-ID and reviewed content; an absent result is not proof
that a message was unsent. Reauthorization needs a dedicated reconciliation path,
without weakening ordinary dispatch's connection-generation fence. Initial compose
currently requires an account already bound to this cache by sync.
Definite authorization or sender-profile failures before queueing now have a
durable Rejected status when the saved review and account bindings match. The
[bounded recovery checkpoint](mail-prequeue-recovery-2026-11-15.md) retains the
original error and keeps existing attempts authoritative. Missing status,
storage failure and invalid/stale bindings remain conservative; they must never
clear an uncertain attempt.

Google's [sending guide](https://developers.google.com/workspace/gmail/api/guides/sending)
and [send method](https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.messages/send)
document MIME/raw and To/Cc/Bcc semantics. Microsoft's
[sendMail method](https://learn.microsoft.com/en-us/graph/api/user-sendmail?view=graph-rest-1.0)
documents MIME and 202 acceptance. These references were checked 2026-11-14;
provider documentation does not establish a live Bruhs send or delivery result.

### Combined validation follow-up, 2026-11-15 UTC

The [combined checkpoint](mail-finalization-2026-11-15.md) records the later
14-step gate and focused copy acceptance. It supersedes pending final-gate/copy
status for that source snapshot; earlier evidence retains its original scope.
Recovery, Sent reconciliation and live-provider/product acceptance remain open.
