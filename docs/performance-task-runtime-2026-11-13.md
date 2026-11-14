# Real task-runtime network measurements

The stream fix removes the snapshot request caused by a real server keepalive. Manual Refresh latency remains similar before and after the change. This verifies request suppression against the frozen older runtime archived for runtime6 (`target/performance-task-runtime4-source`); it does not validate the changing root backend, establish a latency win or prove an equivalent T3 result. The task-workspace feature and this fix remain unpublished working-tree code.

The test uses frozen optimized native `CoordinationRuntime`, JSON persistence, access grants and the paired-task `remote_router`, with real loopback HTTP/TCP SSE. Four completed synthetic tasks each contain eight messages padded with 2,048 ASCII bytes plus labels/Unicode, and there is one shared specification. No provider turns or background workers run. Private counters/revocation controls and a SaveSpec-only command guard are fixture additions. Both pages preserve the same newer task-wake markup; only the stream listener/parser and two disconnect-read guards differ.

## Manual Refresh

Each cell retains three intervals after one warmup. The clock starts at an in-page Refresh click and ends after snapshot DOM updates and two animation-frame callbacks. This excludes navigation/pairing and does not measure a physical pointer or display paint. Fetch duration comes from browser PerformanceResourceTiming and ends with the response body, before JSON parsing/render. Browser transfer sizes are not raw network socket totals.

| Connection | Version | Click through DOM + two frames, median ms | Retained range ms | Fetch through body, median ms | JSON body bytes |
| --- | --- | ---: | ---: | ---: | ---: |
| loopback | before | 25.1 | 23.8–28.8 | 1.8 | 88219–88219 |
| loopback | after | 27.9 | 27.5–28.9 | 1.6 | 88238–88238 |
| mobile_1_6mbps_300ms | before | 779.0 | 778.9–779.5 | 752.1 | 88250–88250 |
| mobile_1_6mbps_300ms | after | 779.2 | 778.5–779.3 | 753.1 | 88238–88238 |

Loopback uses a 1,200×900 viewport without shaping. The mobile test uses a 390×844 touch viewport with Chrome CDP set to 1.6 Mbit/s down, 0.75 Mbit/s up and 300ms latency; CPU is unthrottled. Chrome is 157.0.8103.52. There is no physical device, DNS, TLS or proxy. The two versions share one real runtime. Sequential specification title/revision changes produce the small body-size differences shown above; these are not byte-identical matched workloads. Three samples do not estimate production tail latency.

## Actual keepalive and notification controls

One default Axum keepalive was observed on each version. Its actual browser read contains the three-byte `:\n\n` comment. It causes one snapshot GET before the fix and zero after it. Source inspection confirms Axum's default keepalive interval is 15 seconds. This is one observed case per version, not an hourly production traffic estimate. No exact wasted-response byte or latency total is assigned to these two cases.

Four separate SaveSpec notification diagnostics start by arming the page before an external loopback owner command and end with the latest specification option plus two frames. They include the control round trip and do not isolate server send time. Each is a single interval, not a distribution.

| Connection | Version | Notification diagnostic ms (n=1) |
| --- | --- | ---: |
| loopback | before | 45.5 |
| loopback | after | 45.0 |
| mobile_1_6mbps_300ms | before | 796.1 |
| mobile_1_6mbps_300ms | after | 795.3 |

All 35 controls pass, including exact served-source hashes, native CSP/no-store headers, owner/guest scoping, missing/invalid/expired token and foreign-Origin denial, guest command denial, real SaveSpec persistence, unfinished draft/focus survival, narrow layout, and revoked-stream EOF with reader release. Zero browser JavaScript or console errors occur. All browser contexts close before the native child receives SIGINT and exits with code 0.

The first browser attempt passed 13 native controls, then failed before any browser or timing sample because Playwright's bundled executable was missing. That original failure remains in `target/performance-task-runtime5-browser.json`. The successful attempt uses installed system Chrome. A separate locked build confirmation failed because Cargo wanted to update the private pruned workspace graph; offline confirmation succeeded with all captured source/overlay hashes unchanged and no new package versions. The initial build with unmatched older markup was never launched or measured.

## Remaining scope

- Publish and integrate the complete task-workspace feature with required checks before merging this working-tree performance fix.
- Exercise active/provider work, all task commands, cancellation/reconnect, concurrency and larger histories; match T3 data and boundaries separately.
- Measure DNS/TLS/proxy, physical devices/paint and real impaired links. The current CDP profile shapes loopback transport only.
- Review duplicate startup reads, snapshot payload growth and notification backpressure while preserving lossless current-state delivery.

The broader goal remains active. This study has 20 total UI intervals: 16 retained and four excluded warmups. The 20 normalized timing groups reuse these intervals at different clocks; they are not 20 independent trials. Two separate count-only records describe the actual keepalives. This does not complete every task action, the main browser/desktop app, or any of the seven T3 screenshot comparisons.

The [separate fresh-pairing study](docs/performance-task-pairing-2026-11-13.md) subsequently reuses the initial authorized snapshot, reducing fresh pairing from three GETs to two. It uses a newer matched HTML pair over this same frozen backend. Saved-token startup still uses two GETs; the broader startup/read and integration gaps remain open.

## Evidence

- [performance-task-runtime6-browser.json](target/performance-task-runtime6-browser.json)
- [performance-task-runtime6-run-receipt.json](target/performance-task-runtime6-run-receipt.json)
- [performance-task-runtime6-input.json](target/performance-task-runtime6-input.json)
- [performance-task-runtime6-timings.json](target/performance-task-runtime6-timings.json)
- [performance-task-runtime6-observations.json](target/performance-task-runtime6-observations.json)
- [performance-task-runtime6-counts.json](target/performance-task-runtime6-counts.json)
- [performance-task-runtime4-ui-pair.patch](target/performance-task-runtime4-ui-pair.patch)
- [performance-task-runtime5-build-completion.json](target/performance-task-runtime5-build-completion.json)
