# Mobile access to the desktop session

Phone access now reaches the running native desktop's project chats over authenticated HTTPS. A paired device can read history and live text, send or queue a prompt, answer a specific tool approval, and review the Git working-tree diff associated with the selected thread. This is a local working-tree implementation; it has not been published as a desktop release.

## Connect a phone

1. Configure an HTTPS address the phone already trusts and can reach over Wi-Fi or a tailnet. The desktop supports an existing trusted PEM certificate/key, or a loopback backend behind an existing HTTPS proxy such as [Tailscale Serve](https://tailscale.com/docs/features/tailscale-serve). Proxy mode requires a loopback bind. Bruhs does not install certificates or configure the proxy.
2. In the native desktop's Agent workflows page, open Phone access. Enter the HTTPS origin and listen address, choose the TLS mode, and start access. The proxy-mode default backend is `127.0.0.1:7411`.
3. Choose the device's scopes and lifetime, then show the pairing QR. Read chats is required; prompt, approval and diff scopes start off. Lifetime is 1–24 hours, defaulting to eight hours. The one-use QR expires after five minutes. Closing, replacing or leaving its page invalidates the offer.
4. Scan the QR or open its link in the phone browser. Give the device a name and pair. The [native companion](native-mobile-app.md) consumes the same desktop API and has separate client validation.

The laptop must stay awake with Bruhs running. Stop phone access ends the listener immediately. Restarting it in the same window can restore unexpired, unrevoked devices. Closing that window or restarting the desktop requires pairing again. Devices cannot transfer their authority to another native window.

| Scope | Access |
| --- | --- |
| Read chats | All current and future project chats in this window, including message text, pending tool reviews and live output. Text may itself contain sensitive material. Nonproject chats are excluded. |
| Send prompts | A plain prompt through the selected chat's existing send/queue path. Laptop drafts and context chips are preserved. Desktop slash commands remain laptop-only. |
| Approve tools | Allow or deny the exact displayed card, bound to thread, turn generation, request ID and review hash. Incomplete reviews can be denied; allowing them requires the laptop. |
| Review diffs | A bounded, paged Git working-tree preview resolved from the native thread. The phone cannot supply an arbitrary folder or apply Git changes. |

Private credential forms remain laptop-only. The listener exposes these project-chat projections rather than private Finance, Health, meeting-recording, terminal or arbitrary-file APIs. Existing agent permissions still govern prompts. Model/role changes, hosted relays and push delivery remain outside this ticket's v1.

## Authentication and delivery

The QR contains a 256-bit random secret in its URL fragment. The browser removes the fragment before exchange; the desktop atomically consumes the secret and issues a different random device token. QR encoding uses the [Nayuki Rust library](https://github.com/nayuki/QR-Code-generator/tree/master/rust). Every private API and stream requires its device token, including behind a loopback proxy. Host/Origin checks reject ambiguous headers and constrain the configured origin; they do not replace token authentication.

The native session owns a dedicated device catalog beside the desktop state file. It saves token digests, device names, scopes, expiry, last-seen metadata, revocations and sequence receipts. It contains no raw token, prompt or approval input. A single owner holds the catalog lock. Atomic writes are serialized even when the awaiting HTTP task is canceled.

Revocation denies access synchronously and wakes idle streams before its disk save. Native dispatch rechecks access under the same lock before accepting work. A failed persistence operation blocks mobile access for the entire desktop process. Fix storage, restart and pair again; a new window/process identity rejects old disk tokens. Revocation cannot undo an already accepted desktop action.

Commands carry a UUID, monotonically increasing device sequence and immutable action. Their high-water mark reaches disk before native dispatch, and completed receipts are saved before HTTP success. An exact retry of the latest sequence returns pending or its recorded outcome; older sequences are rejected. A command cannot advance while its predecessor is still being saved. After a listener restart, an unresolved reservation is unconfirmed: delivery could not be established, so check the laptop before deciding what to do next. That command cannot replay. Tool approval acknowledges native acceptance; the existing provider transport may still fail and reports that failure on the laptop.

The browser keeps its device token in that origin's local storage. Before dispatch it saves the exact unresolved command in tab storage; storage failure blocks dispatch. Reload and reconnect reconcile that command with the desktop receipt. Terminal receipts clear it. Unsent phone drafts stay in memory by thread and survive diff/message review; they do not survive a page reload. Disconnect revokes the device and clears its local token and displayed data. Offline state retains marked stale data with mutation controls disabled; learning of revocation or expiry clears the data. An offline phone can retain already received data until it reconnects; the server rejects further access immediately.

## Live views and diffs

An authenticated SSE connection emits refresh hints at most once per second. The browser coalesces them into one current snapshot request at a time and resnapshots after reconnect. Snapshots are limited to 32 KiB, history pages to 50 rows, and text pages to 4 KiB with UTF-8 boundaries. Full-text and diff pages carry revisions; changed content requires reloading. The native queue admits 16 requests with a 15-second delivery deadline. These are payload/concurrency bounds, not measured total heap use.

Diffs use temporary sanitized Git metadata with a private index snapshot and object alternates. Original `.gitignore`, repository exclusions and effective custom global ignore configuration remain effective. Nested project folders resolve their Git root; linked worktrees and split indexes are supported. External diff, textconv, clean/process filters and fsmonitor helpers cannot be introduced through the original live configuration. The user's index is not written. Untracked reads use confined regular-file handles on Unix and do not follow links. On other platforms, untracked file contents receive an explicit omitted-file note; tracked diffs remain available. The preview caps output at 2 MiB and 50 untracked files; each copied index/ignore file also has a 2 MiB budget. Truncation or an unavailable preview directs review to the laptop.

## Verification

Evidence is in `bench/mobile-access/results/2026-11-13/validation.json` and its logs/screenshots.

| Check | Result |
| --- | --- |
| Shared model | One UTF-8 paging test passes. |
| Core host | 24 tests pass. The one ignored helper is exercised by its parent in an isolated Git HOME. Tests cover one-use pairing, scopes, expiry, owner isolation, persistence failure/cancellation, retry races, revocation, real HTTPS trust, idle-stream closure, broker admission, Git helpers, exclusions, nested worktrees and split indexes. |
| Native GPUI and HTTPS | Two tests pass against actual Workspace/ChatView state, covering history/live text, queue/draft preservation, private-form exclusion, paged text and stale/local/turn-changed/incomplete approvals. |
| Phone browser | 33 checks pass in Chromium 158.0.8127.18 at 390×844 touch and 640×480 touch, against that actual native broker over HTTPS. They include response loss/reload, two-tab recovery, live output, inert HTML, diffs, offline/reconnect, independent scopes and revocation. |
| Strict Rust checks | All-target Clippy passes for models, core and desktop with `-D warnings`. |
| Reactive ownership regression | 67 tests pass, two ignored. Last-owner unlock is deterministic despite an inherited open-file description; canceled blocking database work retains ownership until it finishes. |

Transport fixtures verify an isolated test CA with a real Rust TLS client. Chromium pins only the test leaf, and Playwright's request helper trusts that test CA. No system CA was installed and no real user device was paired. Browser evidence proves the actual native state bridge through the GPUI harness; it does not establish a launched desktop's GPU rendering, physical camera scanning, a deployed user's certificate/proxy configuration, or every mobile browser's background lifecycle. The native companion's simulator and release limits are documented separately.
