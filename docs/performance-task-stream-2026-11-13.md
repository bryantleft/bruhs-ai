# Task-stream network fix

The working-tree task UI now reloads its snapshot only for complete refresh events. It ignores SSE keepalives and unrelated events, and coalesces refresh notifications delivered in one chunk. A disconnect also invalidates pending snapshot replies, so late success or failure cannot restore old data or connection status. The change is applied to the unpublished task-workspace code and has not merged to main.

The exact HTML was tested in isolated Chrome 157.0.8103.52 with real loopback HTTP snapshot requests. A controlled browser ReadableStream supplies exact SSE chunks. This tests client framing and request amplification; it does not reproduce TCP segmentation, the real task runtime/provider, impaired networks or action latency. The synthetic snapshot contains one shared specification and no tasks.

The [separate actual-runtime study](docs/performance-task-runtime-2026-11-13.md) subsequently verifies real HTTP/TCP SSE, keepalive request suppression, access/persistence controls and bounded loopback/CDP refresh timings on a newer frozen source. Its data and clocks are separate from this controlled-fragmentation study.

## Snapshot requests caused by stream cases

| Stream input | Delivered chunks | Baseline requests | Candidate requests | Baseline response body bytes | Candidate response body bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| keepalive comment | 1 | 1 | 0 | 69806 | 0 |
| one-byte fragmented refresh | 30 | 30 | 1 | 2094180 | 69806 |
| two complete refreshes in one chunk | 1 | 1 | 1 | 69806 | 69806 |
| unknown data event | 1 | 1 | 0 | 69806 | 0 |
| UTF-8 comment split into bytes | 14 | 14 | 0 | 977284 | 0 |
| CRLF refresh split into bytes | 33 | 33 | 1 | 2303598 | 69806 |
| CR refresh split into bytes | 30 | 30 | 1 | 2094180 | 69806 |
| long data line | 1 | 1 | 1 | 69806 | 69806 |

Across these eight event cases, snapshot requests fall from 111 to 5. Candidate desktop, 390px viewport and local-listener cohorts agree. Body bytes count the synthetic JSON responses, excluding HTTP/TCP overhead; they are not real production payload totals. No elapsed-time speedup is claimed.

## Verification

The parser passes 508 reference controls across 16 fixtures, including every two-part split point for short fixtures, bytewise UTF-8, BOM, LF/CRLF/CR, empty chunks, long data/comments, unknown events and incomplete EOF. It retains only the meaningful control prefix rather than complete long lines.

The main browser test passes 32 stream cases and 21 behavior checks. Nine separate adversarial checks cover local/bearer request headers, latest rendered revision, late old-listener EOF, reader release and 390px layout. Two additional controls fail on the earlier candidate and pass after pending snapshot success/error are invalidated on disconnect. All test browsers, contexts and servers close.

The initial browser gate failed because it treated three intentional invalid-token HTTP 403 console messages as unexpected errors. That report is preserved. The corrected harness verifies each expected 403 against its deliberate request and rejects all other browser errors; the final run has zero unexpected errors.

## Remaining scope

The later actual-runtime study verifies a limited SaveSpec/keepalive/access path and defined refresh clocks. Active/provider work, other task mutations, complete cancellation/reconnect flows and broader latency remain unverified. This is a verified client/request-count fix, not whole-action or faster-than-T3 acceptance. A 390px viewport check does not establish mobile equivalence. Required integration checks remain before a main merge.

## Evidence

- [Applied source](bruhs-server/src/routes/coordination/remote.html)
- [Patch](target/performance-task-stream2-candidate.patch)
- [Main browser results](target/performance-task-stream2-browser.json)
- [Adversarial controls](target/performance-task-stream2-adversarial.json)
- [Disconnect success regression](target/performance-task-stream2-disconnect-red-success.json)
- [Disconnect success fix](target/performance-task-stream2-disconnect-green-success.json)
- [Application receipt](target/performance-task-stream2-application-receipt.json)
