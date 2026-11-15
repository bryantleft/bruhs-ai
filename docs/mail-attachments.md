# Mail attachment downloads

Status: implemented on web/native; controlled runtime, HTTP, native UI and
production browser checks pass. This does not establish
live-provider, send, delivery or full Inbox completion.

The user explicitly selects an attachment in a saved connected thread. The server
checks its own host, current broker generation and mailbox before any provider
read. The Gmail adapter resolves the canonical attachment inside that account's
cached message. A caller cannot substitute a provider URL or an unrelated message
or attachment ID.

Separately stored parts use Gmail's attachment endpoint. Embedded parts fetch
only their owning message, validate message/thread identity and reject ambiguous
MIME part IDs. Base64 decoding and both declared/decoded sizes must agree with
the saved metadata. Empty files are valid; a MIME container with children and no
file bytes is rejected. Missing, malformed, truncated, limited
or mismatched replies do not change cached mail, cursors or drafts. Response and
MIME-depth limits remain the same as the read/sync adapter.

The HTTP response is always an explicit binary download with `no-store`,
`application/octet-stream`, `attachment` and `nosniff`. Neither a filename nor an
email's claimed MIME type can turn it into an inline application document or
inject response headers. The app never opens or executes the downloaded file.
The provider contract is Google's [attachment endpoint](https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.messages.attachments/get)
and [MIME part body schema](https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.messages.attachments),
checked 2026-11-14.

Native first opens the OS save dialog. Canceling it reads no attachment bytes.
Only the basename is suggested; the user owns the destination and overwrite
review. The client validates the complete saved size before creating a private
temporary file. A blocking worker owns the file and cleanup through write, sync
and atomic rename; Unix files use 0600. A failed network/provider retry preserves
the previous destination. The UI reports saved only after the write finishes.

Web keeps the bytes in a browser-owned Blob, validates its exact size and starts
a local Blob download with a sanitized basename. It removes the temporary anchor
and revokes the object URL. The app owns the job across page navigation, prevents
duplicate download jobs and clears an earlier success notice before a new action.
Its success copy says that download started and directs the user to browser
downloads for completion. Both clients resolve current Gmail connection metadata
instead of treating a cached connection ID as authorization.

The shared attachment buffer ceiling is 64 MiB; the provider's JSON transport
also has its existing 64 MiB encoded-response ceiling. Large-file usability and
performance remain separate acceptance work. No attachment is fetched simply
because a reader, inline image or metadata row becomes visible.

The exact targeted checkpoint is
`target/inbox-ui-qa/mail-attachments-integration/validation-20261114T163124Z/`:
83 core mail tests, four production-router/native-client HTTP tests, five native
Inbox tests, and 1,009 model/web tests with six ignored. Strict native and actual
WASM lint, dependency policy, formatting and whitespace checks pass, with stable
source hashes and a 1,007-file source/log manifest. The GPUI fixture exercises
cancel and selected-path saves; physical OS dialog presentation and its suggested
name field are outside that fixture's API.

The later container guard passes 84 core mail tests and four HTTP tests, with
strict Core/Server lint and stable source hashes recorded under
`target/inbox-ui-qa/mail-attachments-integration/container-guard-20261114T165127Z/`.
Native feedback geometry checks and strict native lint pass after moving status
above the reader's scrolling content.

The corrective default production capture is
`target/connected-mail-ui-dist-20261114T170006Z/`. Its 1,525 checks retain the first
attachment capture's 1,327 assertions and add 198 feedback checks: status remains
visible during reader scroll, and long regions cap at 128px with keyboard access.
All 36 actual Chrome downloads (25,194,738 bytes) match bytes, SHA256 and filenames;
all 36 Blob URLs and anchors are released. Coverage includes empty and 8 MiB + 17
byte files, inert HTML, a 64 MiB + 1 metadata refusal before any attachment read,
navigation during pending work, duplicate prevention, explicit errors and retries.
The browser's largest successful fixture is not the full buffer ceiling.

Receipts, 96 screenshots, network/file reviews and the 193-file manifest are under
`target/inbox-ui-qa/connected-mail-ui-20261114T170006Z/browser-20261114T171149Z/`.
The prior capture at `target/connected-mail-ui-dist-20261114T162622Z/` keeps its
1,327/0 and setup 642/0 results plus its manual feedback-placement limitation.
All owned browser resources closed; the fixture-only failed attempt is retained.

Physical macOS acceptance subsequently opened the actual save dialog, canceled
before any byte request, then saved 285 exact synthetic bytes with mode 0600 and
the canonically equivalent Unicode filename. Exact binary hashes, screenshots
and the filesystem receipt are under
`target/inbox-ui-qa/native-physical-20261114T172358Z/`. A native-only correction
exposes reader/feedback text to the AX tree and bounds keyboard-scrollable status
without reserving empty space for short results. Its five Inbox tests, two static
accessibility audits and strict native/WASM checks pass. Actual window checks
cover 1,280×800 and 640×480. Full keyboard/VoiceOver, scaling, motion, other devices
and live Gmail remain separate acceptance work; both QA apps and fixtures closed.

Earlier
[read/sync acceptance](connected-mail-ui-2026-11-14.md) keeps its original source
and artifact scope. Compose/reply, provider drafts, Outlook, live account recovery,
real recipient delivery and the wider product goal remain open.
