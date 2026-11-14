# Fresh task-device pairing

Fresh pairing now uses its first authorized snapshot immediately instead of downloading the same state again before showing tasks. It still refreshes after subscribing to the event stream, so changes between the first read and subscription are caught. On the controlled slow connection, the first usable view appears about 0.75s sooner. The fix is applied to the unpublished task HTML and remains unmerged.

Pairing and snapshot reads have separate ownership counters. A delayed pairing success/error cannot restore a disconnected session or clobber a newer pairing attempt. Manual Refresh can run while pairing is pending without cancelling the pairing or allowing its older snapshot to overwrite fresher results. Successful pairing also clears its prior error.

## Measured boundaries

Each profile/version retains three intervals after one warmup, with alternated ordering and fresh browser sessions. The clock starts at normal in-page form submission after navigation and token entry. First usable means the authenticated task DOM is present and the pairing form is hidden. Initial settlement additionally waits for the initial stream refresh to parse/render and the next actual SSE read to be pending. The primary endpoints add two animation-frame callbacks; neither proves physical display paint.

Both versions use the same unchanged 88,264-byte owner snapshot from the same frozen optimized native runtime. Real authentication, JSON persistence and HTTP/TCP SSE run through identical private loopback forwarding adapters. The adapters replace only the HTML body during measurements; response holds are used in separate race controls. The actual backend is the older runtime6 source archive, not the changing root backend. No provider turns or background workers run.

| Connection | Version | First usable + two frames, median ms | Range ms | Initial settled + two frames, median ms | Range ms |
| --- | --- | ---: | ---: | ---: | ---: |
| loopback | before | 31.8 | 30.4–44.7 | 48.5 | 44.7–63.9 |
| loopback | candidate | 36.6 | 27.7–38.7 | 55.3 | 54.5–61.3 |
| mobile_1_6mbps_300ms | before | 1531.8 | 1530.5–1538.5 | 2613.2 | 2604.7–2615.2 |
| mobile_1_6mbps_300ms | candidate | 781.9 | 780.3–782.4 | 1848.5 | 1846.9–1849.4 |

The loopback two-frame medians are slightly higher for the candidate, with overlapping observed ranges. Loopback first-DOM medians are 12.6ms before and 10.3ms after. Keep those results separate from the strong impaired-link improvement; this is not a universal latency win. With three retained samples, nearest-rank p95/p99 are maxima and do not estimate production tails.

Loopback uses 1,200×900 without shaping. The mobile profile uses 390×844 touch mode, 1.6 Mbit/s down, 0.75 Mbit/s up and 300ms CDP latency, with unthrottled CPU. Chrome is 157.0.8103.52. This is browser shaping over loopback, without a physical device, DNS, TLS or a production proxy.

## Network work

| Fresh pairing | Snapshot GETs | Snapshot JSON body bytes |
| --- | ---: | ---: |
| Before | 3 | 264792 |
| Candidate | 2 | 176528 |

Every retained trial agrees on these counts and body totals. They exclude navigation HTML, headers/TCP overhead and the open SSE stream; browser ResourceTiming body sizes agree with adapter byte counts. The candidate HTML is 270 bytes larger, outside these pairing clocks. Saved-token startup remains two GETs in both versions. Its initial refresh cannot be dropped without accounting for state changes between snapshot and subscription.

## Verification

Independent browser verification passes all 23 candidate checks. Across 51 checks, four baseline checks intentionally reproduce failures: a stale retry error, delayed success after Disconnect, delayed invalid reply after Disconnect, and an old invalid reply clobbering newer guest pairing. There are zero unmatched console, JavaScript, HTTP or harness errors. Intentional invalid-token responses are matched by phase/request. Hidden-button Disconnect is explicitly an adversarial event-handler control.

Real SaveSpec changes made after a snapshot is captured and before subscription appear through the mandatory initial refresh. A separate held-pair/manual-Refresh control preserves newer data and still completes pairing. Draft, focus, selected task, 390px layout, owner/guest scope, expiry, exact persisted state and acquired-reader release pass. All 16 verification contexts, Chrome, forwarding adapters and the native child close; the native child exits 0. The project's existing local/guest browser flow also passes after application.

The timing run is separate from these mutation controls and uses unchanged data. Its 16 total intervals comprise 12 retained intervals and four warmups. The 16 normalized timing groups reuse those intervals at four clocks and are not additional independent trials. All timing contexts, browser, forwarding adapters and native child close; native exit is 0. All prior log observations remain preserved.

## Remaining scope

- Integrate and verify the complete task-workspace feature before a main merge. The changing root backend and the full application are not validated by this older frozen native build.
- Saved-token startup still reads two snapshots. Removing its initial stream refresh needs a protocol/order proof so intervening changes are never lost.
- Exercise provider/active tasks, larger histories, reconnect/error/cancel/concurrency variants and every other task action.
- Measure physical devices/paint, DNS/TLS/real proxies and equivalent T3 workloads; keep unmeasured actions explicit.

The broader performance goal remains active, with no whole-action or matched T3 acceptance from this study.

## Evidence

- [performance-task-pair1-candidate.patch](target/performance-task-pair1-candidate.patch)
- [performance-task-pair1-application-receipt.json](target/performance-task-pair1-application-receipt.json)
- [performance-task-pair1-verify01-browser.json](target/performance-task-pair1-verify01-browser.json)
- [performance-task-pair1-verify01-final-readback.json](target/performance-task-pair1-verify01-final-readback.json)
- [performance-task-pair1-timing1.json](target/performance-task-pair1-timing1.json)
- [performance-task-pair1-run-receipt.json](target/performance-task-pair1-run-receipt.json)
- [performance-task-pair1-observations.json](target/performance-task-pair1-observations.json)
- [performance-task-pair1-project-browser.stdout.log](target/performance-task-pair1-project-browser.stdout.log)
