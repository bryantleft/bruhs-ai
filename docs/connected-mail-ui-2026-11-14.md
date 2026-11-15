# Connected Email integration

Status: read/sync and attachment downloads pass production browser checks and
controlled native checks. These are scoped checkpoints, not completed Inbox or
live-provider acceptance.

Inbox now has an Email entry on web and desktop. Imported conversations keep
their existing local search, filters, drafts, persistence and agent actions.
Email reads the canonical private server cache through the real mail HTTP API;
desktop creates no second mail store or sync daemon.

Saved account selection, full-text search, the shared Inbox filters and bounded
100-thread pages use one consistent server cache snapshot per response. Search
reaches complete older bodies, recipient headers and attachment names, rather
than only the row preview. Stale page numbers return the last available page.
Rows show the actual counterparty and date. The reader exposes every saved
message through Older/Newer controls, sender/recipient metadata, attachment
metadata and selectable or explicitly copyable plain text. It does not execute
HTML or load remote resources.

Sync resolves the server's current Gmail connection generation before starting.
It checks the host and selected account on each page. The app owns the job across
page navigation; Stop prevents the next page after the current atomic page ends.
Refresh rereads committed cache state without clearing a provider sync failure.
Retry starts from the saved checkpoint, and explicit Restart sync retains mail
until a complete scan can reconcile it. No read action sends a message.

The source checkpoint at
`target/inbox-ui-qa/mail-browse-integration/checkpoint-20261114T155143Z/`
contains exact sources, raw logs and a SHA256 manifest. It records 78 core mail
tests, three tests using the production HTTP router and native client, five native
Inbox tests, and 1,008 model/web tests with six ignored. Strict native and actual
WASM Clippy, dependency policy, formatting and whitespace checks pass. A native
debug-selector clone lint was corrected and its Inbox tests rerun; failed earlier
attempts remain in the evidence.

The first default production bundle is
`target/connected-mail-ui-dist-20261114T154105Z/`: 298 assets, API origin
`http://127.0.0.1:3001`, with stable source hashes during capture. Its source
snapshot and build receipt are under
`target/inbox-ui-qa/connected-mail-ui-20261114T154105Z/`. Earlier accepted Inbox
and Today bundles retain their original evidence and version limits.

The corrective default capture `target/connected-mail-ui-dist-20261114T155735Z/`
passes all 601 connected Email assertions across desktop, compact and touch. It
fixes nine first-run failures in one long-account-label overflow family, retaining
every assertion identity and the same harness. First-capture Inbox 399, synthetic
setup 642 and original Today 467 regressions also pass, with that earlier version
scope retained. The corrected matrix has 12 cases and 51 screenshots; the parent
and browser verifier inspected the key layouts. Exact sources, build inputs,
runtime/network review and the 121-file manifest are under
`target/inbox-ui-qa/connected-mail-ui-20261114T155735Z/browser-20261114T160526Z/`.
All owned browser resources closed. The attachment batch subsequently separated
the count from the wrapping account name.

Attachment downloads have their own [contract and checkpoints](mail-attachments.md).
The first attachment source checkpoint passes 83 core mail tests, four
actual-router/native HTTP tests, five native Inbox tests and 1,009 model/web tests
with six ignored. Strict native/WASM lint and dependency/format checks pass.
The later MIME-container guard passes 84 core and four HTTP tests; it rejects a
container with no file bytes instead of saving a false empty attachment. Native
tests exercise cancellation before byte access, exact bytes, private atomic save
and preservation of an existing file after a failed retry. Physical OS dialog
presentation and its suggested-name field are outside that fixture's API.

The attachment capture `target/connected-mail-ui-dist-20261114T162622Z/` passes
1,327 browser checks and 642 setup checks. Manual screenshot review nevertheless
found feedback above the reader's scrolled viewport. That result retains this
visual limitation. The corrective default capture
`target/connected-mail-ui-dist-20261114T170006Z/` places bounded, keyboard-scrollable
feedback before the reader scroller. It passes 1,525 checks across 21 cases and
96 screenshots, retaining all 1,327 assertions and adding 198. All 36 browser
downloads match exact bytes, hashes and sanitized filenames; Blob URLs and
temporary anchors are cleaned up. Errors, pending state and download-start
results stay visible on compact and touch layouts. Browser success means a
download started; completion was independently checked in the synthetic fixture.

Exact build inputs, unchanged-source checks, network review and the 193-file
manifest are under
`target/inbox-ui-qa/connected-mail-ui-20261114T170006Z/browser-20261114T171149Z/`.
The first strengthened attempt's two fixture-only failures remain archived; no
product assertion was removed. All owned browser resources closed. Earlier
Inbox, Today and setup results keep their recorded bundle scope; these repeats
do not add unique product coverage or establish a current whole-workspace gate.

The actual macOS GPUI application was then exercised at 1,280×800 and 640×480
with an isolated temporary workspace and controlled HTTP transport. The OS save
dialog suggests the sanitized Unicode basename; cancel reads no attachment bytes.
A completed save preserves the exact 285 fixture bytes with mode 0600. The native
filesystem returns decomposed Unicode, so filename comparison uses canonical
equivalence. Search submitted by Enter reaches an older message body.

This physical pass found reader text missing from the accessibility tree and
short feedback reserving its full 128px limit. The native correction exposes
subject/body, envelope metadata and feedback through GPUI's accessibility API,
uses natural height for short results, and keeps long feedback bounded with
Home/End and arrow/page scrolling. Five focused Inbox tests, two accessibility
audits and strict native/WASM lint pass. The corrected actual window exposes the
reader and quota error text in its AX tree. Receipts, original findings, binary
hashes and screenshots are under
`target/inbox-ui-qa/native-physical-20261114T172358Z/`. The save-dialog proof belongs
to the earlier captured native binary; the corrected renderer keeps that exact
HTTP client/save implementation. Both owned app runs and fixture servers closed.
This focused capture does not freeze the complete native dependency graph or
establish VoiceOver/full keyboard acceptance. The web bundle is unchanged.

Open work includes broader keyboard/device and accessibility acceptance,
performance, client replicas while the server is
unavailable, restart restoration of connected browse state, People/triage
overlays, compose/reply, Outlook, live account recovery and
verified recipient delivery. The existing reviewed-send journal is not a send
implementation. The [compose contract and local draft storage](mail-compose.md)
track that next implementation slice. The all-pillar goal and the separate whole-workspace gate remain
open. Git index/history and the held sidebar patch are unchanged.
