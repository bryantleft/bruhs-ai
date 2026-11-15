# Gmail read/sync implementation

Status: **in progress**, 2026-11-14. The native adapter reads through the real
HTTP client and canonical mail runtime. Its transport tests use isolated
loopback Gmail fixtures and synthetic broker tokens. Inbox UI transport is now
[wired on web and native](connected-mail-ui-2026-11-14.md). Real
account acceptance, provider writes, send/reply, full attachment acceptance and Outlook
remain open; this is not a completed mail client.

`GmailClient` uses a fixed Gmail API origin, current broker access under its
session lease, bounded responses and disabled redirects. Provider errors never
include raw response text or credentials in public diagnostics. Content is data:
no scripts, remote images or external HTML resources are executed or loaded.

Full sync captures a profile history baseline before scanning every thread-list
page. Each listed thread is fetched in full, including all messages and any
separately stored text bodies. The checkpoint and that page's records commit
together. An indexed server-side seen set survives restart; missing threads are
pruned only when the complete scan commits. Independent drafts remain. The
original baseline then drives history catch-up, covering changes during the scan.
The 50-thread page size bounds one step; it is not a mailbox-history cutoff.

History pages keep the starting baseline while a next-page token exists, then
advance to the final returned history ID. IDs compare as arbitrary-length decimal
strings. Repeated message/label changes fetch each affected complete thread once.
A confirmed thread 404 removes its cache record; a missing body part instead
fails the update and retains the thread/cursor. Expired history starts a new full
scan. Explicit restart can recover a malformed/full-page checkpoint without
prematurely erasing mail or drafts.

Provider-neutral Inbox content is stored with explicit connected provenance,
canonical message/attachment IDs, complete recipient headers and attachment
metadata. Imported Email labels never establish ownership. Encoded names and
subjects use the pinned MIME parser; declared text charsets are decoded. Plain
alternatives are preferred, with HTML converted to plain text when necessary.
Attachment metadata stays separate from the body, including inline content IDs.
Explicit [attachment downloads](mail-attachments.md) are a separate user action;
they do not happen during sync or rendering. Compose is still open.

Limits remain explicit: each provider response is capped at 64 MiB, MIME nesting
at 64, and a header block at 1 MiB. Malformed, truncated or unsupported content
fails without advancing the sync position; it is not silently accepted as a
complete thread. Large-mail usability and performance still require acceptance.
Custom Gmail label names, provider drafts, local People/triage overlays, credential
recovery during long operations and change feeds remain open. Explicit sync can
stop between atomic pages and survives navigation; device and live-provider
acceptance remain separate. Current quota errors retain Retry-After hints; domain policy is
distinct from mail consent, and unknown permission errors also mention API setup.

The original integrated read/sync checkpoint passes 67 mail/runtime checks. The
additional quota/policy and cursor-recovery regressions pass in a 71-test run;
strict current-source Core/Server/WASM lint, dependency and formatting checks pass.
Exact source copies, hashes and commands are retained under
`target/inbox-ui-qa/gmail-integration/checkpoint-71/`. Tests use actual HTTP, broker leases and
disk SQLite, not a replacement persistence implementation. No live account or
recipient-delivery result is implied. See [runtime ownership](mail-runtime.md)
and the [Inbox acceptance cases](inbox-completion-2026-11-13.md).

The provider contract follows Google's [sync guide](https://developers.google.com/workspace/gmail/api/guides/sync),
[thread listing](https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.threads/list),
[full-thread reads](https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.threads/get),
[message/MIME schema](https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.messages),
[history pagination](https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.history/list),
and [error guidance](https://developers.google.com/workspace/gmail/api/guides/handle-errors),
checked 2026-11-14. The [pinned MIME parser API](https://docs.rs/mail-parser/0.11.9/mail_parser/)
supports the repository's current Rust toolchain.
