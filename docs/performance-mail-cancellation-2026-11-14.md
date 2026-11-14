# Mail navigation cancellation checkpoint

The tested private production capture cancels abandoned web mail HTTP reads after Back, replacement search or account selection. Its two Rust files and a necessary browser-test correction are retained in the shared working tree; the later combined root graph still needs fresh verification. The performance goal remains active: this checkpoint adds response-body diagnostics, not complete-action latency or a matched T3 win.

Previously, generation checks kept stale results out of the UI while the old requests continued downloading. The fix gives browse and detail reads separate app-owned AbortControllers. A newer read, selection reset or Back cancels the corresponding request; a finished request releases its handle only after its generation check. Existing sync, page-boundary Stop and explicit download behavior are preserved.

| Transition | Response stage | Before: body bytes | With cancellation: body bytes |
| --- | --- | ---: | ---: |
| Back, then another thread | Before response headers | 263,294 | 0 |
| Back, then another thread | During body | 263,294 | 16,384 |
| Replacement search | Before response headers | 262,759 | 0 |
| Replacement search | During body | 262,759 | 16,384 |
| Account switch | Before response headers | 263,294 | 0 |
| Account switch | During body | 263,294 | 16,384 |

Each row is one diagnostic per implementation. The valid JSON fixtures contain 256 KiB of ignored padding; a gate permits 16 KiB before body-phase navigation. The underlying full fixture bodies match between implementations by SHA256 and length; transferred prefixes differ. Native Chromium fetch sends real GETs to an isolated loopback server; interception rewrites only the API destination host/port. Server writes and CDP body counts agree. Requests close before fixture release and browser teardown, and no stale body bytes are emitted after the latest view is ready.

These counts exclude request/response headers and TCP/TLS overhead. The padding and gate make this a cancellation proof, not a representative workload or bandwidth benchmark. Diagnostic clocks remain separate and unnormalized. Real providers, DNS/TLS/proxies, physical devices, complete-action latency and all seven T3 comparisons remain unaccepted. [Independent byte replay](target/performance-mail-cancel1/network-body-observations.json).

## Validation and source

- The private default release graph matches all 607 captured inputs before applying the two-file fix: 605 remain unchanged, and two differ. Three newer root inputs were restored from exact SHA-matching backups. The private package path differs from the baseline. [Input reconstruction](target/performance-mail-cancel1/source-seed.json).
- All 440 web tests pass, with five ignored. Strict host and actual-WASM Clippy pass with all targets/features. The default release contains 298 assets, API 3001 and `NO_COLOR=true`. Missing benchmark parse-support files caused an initial failure before compilation; offline Trunk tool lookup later failed after optimized Rust compilation. Those failures are retained; the successful retry reused Rust and resolved Trunk tools normally. [Checks and build](target/performance-mail-cancel1/checks-build03.json).
- The transport replay passes 1,993 checks. All six cases cancel the original AbortSignal and close real HTTP responses; eight extra replacement searches also cancel correctly and preserve the final view. [Probe](target/performance-mail-cancel1/candidate-probe/result.json).
- Gmail passes 1,595 checks and Outlook passes 1,771 across 45 fixture cases and 198 screenshots. All 3,240 earlier accepted check identities ran, plus 126 cancellation checks. These are synthetic provider/account/cache/attachment responses, not live-provider acceptance. [Functional results](target/performance-mail-cancel1/candidate-probe/functional/result.json).
- Seventeen unrelated functions and the request timeout/decode policy remain byte-identical, including sync, Stop and downloads. Four copied-evidence controls reject altered body counts, CDP counts, fixture hashes and late closure. [Preserved contracts](target/performance-mail-cancel1/structure-check.json) · [Tamper controls](target/performance-mail-cancel1/negative-controls/result.json).

The existing QA driver waited for HTTP responses from five superseded held GETs. Correct cancellation makes that assumption obsolete. Its updated gate requires the exact original signal, GET URL, request sequence, unique `net::ERR_ABORTED` and absence of a response before fixture release. It then executes every original stale-view assertion. All 122 original check callsites remain; unrelated failures still fail the suite. Browser replay used the private driver and capture. The adopted QA file is syntax-checked and restores the original repo-relative path prelude, with all remaining test bytes identical to that executed driver. A fresh current-root run remains pending. [QA adoption](target/performance-mail-cancel1/qa-adoption.json).

The two production files were adopted only after their full preimages matched, and their final bytes match the tested candidate. The prior eleven audit-owned sources, Git index and sidebar WIP patch are unchanged. The new fourteen-file source checkpoint includes the two Rust files and QA driver. [Production adoption and exact hashes](target/performance-mail-cancel1/adoption.json).

## Action log and remaining work

The proposed catalogue now contains 62 reviewed intents: 45 earlier command intents plus 17 connected-mail intents. Each mail intent names its completion boundary and required variants. This remains a partial semantic review. The last pinned source discovery contains 366 entrypoints and 1,378 binding references; subsequent compose work is outside that snapshot. The stale ledger note saying router discovery still refuses the new merged router has been corrected.

The proposed log preserves all 242 earlier evidence entries, 2,236 timing groups and 34 request-count cases exactly. It adds two partial evidence entries and 12 separately typed response-body observations. Whole-action acceptances and matched T3 wins remain zero. The sealed successors were adopted into the working-tree performance ledger, action log and semantic catalogue by the finalization coordinator after verifying all 432 retained evidence pins and 14 current source pins. The earlier 229 published entries, 2,004 timing groups and 34 request-count cases remain byte-exact as JSON values. This documentation adoption does not validate the later combined application graph. [Action requirements](target/performance-mail-cancel1/proposed-action-catalogue.json) · [Action log](target/performance-mail-cancel1/action-log-preview.md) · [Typed JSON log](target/performance-mail-cancel1/action-log-preview.json).

All experiment-owned build, browser and fixture jobs are closed. Source reservations are released. Only this experiment's private Cargo clone was removed; source, production captures, dependency/GPUI caches and evidence were preserved. No index/history write was performed. The later combined root graph still needs the feature owner's checks and fresh capture; this private graph does not establish acceptance for concurrent compose additions. [Runtime closure](target/performance-mail-cancel1/candidate-probe/completion.json) · [Private-cache cleanup](target/performance-mail-cancel1/private-cache-cleanup.json).
