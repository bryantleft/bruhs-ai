# Connected Email browser verification

The harness runs the captured production WASM in installed headless Chrome with
the repository's existing Playwright fallback. It never builds, installs, opens a
real provider, uses credentials, or sends email. It downloads only explicit
synthetic attachment bytes into QA artifact files.

Use an explicit fresh capture and its successful build/source receipts:

    BRUHS_CONNECTED_MAIL_BUILD_RECEIPT=/absolute/capture/build.json \
    BRUHS_CONNECTED_MAIL_BUILD_INPUTS=/absolute/capture/build-inputs.json \
    NO_COLOR=true node bruhs-web/tests/connected_mail_browser.mjs /absolute/fresh/dist

The default provider fixture is Gmail. Run the same assertions again with
`BRUHS_CONNECTED_MAIL_PROVIDER=outlook` and a fresh output directory to verify
Outlook routing, current authorization, sync/Stop and downloads against the same
captured assets. Both sync controls are checked for accessibility. Outlook also
checks bounded MIME export-size receipts and explicit metadata-only linked
attachments; ordinary file truncation assertions remain unchanged.

The capture receipt must contain a successful exitCode, absolute dist, and an
assets map with sha256 and mtime_ns. The index hash may be top-level indexSha256
or the index.html asset entry. Source receipt hashes
must match every relevant production source. The harness checks all captured
files and exact nanosecond mtimes before launching and checks them again after
completion. Without receipts, the explicit dist, real WASM asset, source
freshness, and postflight invariants still apply; acceptance should supply both
receipts. BRUHS_PLAYWRIGHT_PATH and BRUHS_CHROME can select an existing installed
runtime. Neither variable causes an installation.

Each of desktop 1280×800, compact 640×480, and coarse-pointer touch 390×844 runs
seven isolated scenarios:

- Actual imported form, real IndexedDB draft/search retention, lazy Email
  fetch, two saved accounts, encoded Unicode full-text query, five filters,
  bounded pages, a shrinking result set with a server-clamped final page,
  actual envelope/attachment metadata, all seven saved messages, inert
  script/image text, keyboard Back/row focus restoration, Connections Settings,
  hit targets, overflow, mono typography, and reduced motion.
- Read error/retry, persistent sync error after successful cache refresh,
  current Gmail connection generation, three-page sync, Stop after a held page,
  route unmount while sync continues, return/Stop/resume, and explicit restart.
- Late search, account, and thread replies; host refresh while old rows/detail
  are pending; and the visible server-identity error when sync began before a
  host change.
- Deliberately unavailable network read, retry into an empty-account prompt,
  and disconnected-current-generation recovery without submitting sync.
- Actual browser files for binary, zero-byte inline,8MiB+17byte and HTML
  payloads; exact bytes/SHA256 and sanitized suggested names; a64MiB+1metadata
  guard; current connection generation and encoded attachment identifiers.
- Held connection/binary reads, duplicate/sibling click prevention, busy state
  through route unmount/remount, and download completion off-route.
- Offline,403,409,429 and short200-body failures without file/Blob creation,
  unchanged cached reader, refresh and retry; stale download-started notices
  must clear on a later failure.

Every API/provider response is synthetic. Only the selected explicit local provider-sync
POST is allowed; every other attempted mutation fails the run. Unrelated reads
use a finite allowlist. Held responses remain held until the scenario releases
them. Synthetic cache revisions advance only when a successful page is
released. Runtime errors and unexpected requests fail; console/network errors
are accepted only for the exact injected request URL and status/failure.

Output is timestamped under target/inbox-ui-qa/connected-mail-browser by default.
BRUHS_CONNECTED_MAIL_QA_OUTPUT may name a fresh evidence directory. report.json
contains checks, requests, synthetic responses, held responses, real imported
IndexedDB snapshots, screenshots, and input/postflight hashes.
artifact-sha256.json hashes every output. Nonzero exit means an assertion,
interaction, or integrity failure. Inspect failures and actual screenshots;
do not replace failed product assertions with weaker expectations.

This verifies the compiled client and synthetic transport contracts. It does
not prove actual server full-text search, Gmail synchronization, real account
authorization, native/device parity, provider attachment retrieval, send
delivery, or performance. Existing imported Inbox and service-setup regressions and the
original Today suite should be run separately against the same captured
version when shared app initialization changes.

Passive wrappers call native Blob URL, anchor click and timer APIs without
substituting their results or timing. Each successful download must remove its
real hidden anchor and revoke its Blob URL. Playwright saves case/index.bin QA
files to avoid repeated-name collisions; the Chromium suggestedFilename is
checked separately against the exact sanitized name. A proven successful owned
Blob download may report ERR_ABORTED as a navigation transition. Only that
exact completed-download URL is classified separately, never API failures or
failed files. Native20s/180s timeout registration is observed; full deadline
expiration is not waited. The largest successful file fixture is8MiB+17bytes.

Viewport feedback regression: before each attachment action the reader is
deliberately scrolled. Pending, success, failure and retry feedback must stay
fully inside viewport/ancestor clipping bounds and pass native hit testing.
The feedback locator is never scrolled into view by the test. Its separate
focusable status region must be capped at128px. Concurrent long sync and download
recovery tokens must wrap, overflow inside that region, and expose the final marker
with native keyboard End while preserving the cached reader. The old1327pass
run and its manually observed compact/touch feedback limitation remain archived.
