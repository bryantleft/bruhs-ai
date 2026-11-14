# Task snapshot transport compression

The unmerged working-tree paired-task router is configured to negotiate gzip Fastest for successful finite responses whose uncompressed bodies are at least 768 bytes. Smaller replies bypass compression. Event streams, images and error responses stay uncompressed. The full task-workspace feature also remains unmerged.

A frozen three-option experiment reduced repeated 88,390-byte snapshot bodies to 1,860 bytes with Fastest gzip. Its shaped-link Refresh median fell from 780.4ms to 346.9ms. The independently varied ASCII fixture fell from 780.8ms to 596.7ms. These are bounded prototype results over an older archived backend, not current-root, physical-device, whole-action or matched T3 acceptance.

## Experiment and clocks

The timing experiment uses the earlier 32-byte compression floor. Identity, Fastest and Default run in one optimized binary over the real CoordinationRuntime, authentication, JSON persistence and TCP SSE. Every variant serves the same f335 task HTML and reads the same unchanged snapshot per payload profile. No provider turns or background workers run. The five payloads include small controls, repeated text, independently seeded ASCII text and a larger repeated history.

The matrix retains 480 native HTTP intervals after 75 warmups and 90 UI intervals after 30 warmups: 675 total intervals, 570 retained. Native cells retain 32 samples; UI cells retain three. The 120 normalized timing groups reuse these intervals at different clocks and are not additional independent trials. Nearest-rank p99 at n=32 and p95/p99 at n=3 are maxima, not production tail estimates.

Native time starts at Node http.request construction and ends at the encoded response end event. It includes server cloning/serialization/compression, client accumulation and loopback transport; concatenation, gunzip and validation follow the clock. It is not isolated server CPU. Node gunzip has a separate clock. UI time starts at an in-page Refresh click and ends after DOM update plus two animation frames. Navigation, initial reads and pairing precede it; no physical pointer or display-paint claim.

Mobile shaping uses 390×844 touch mode, 1.6 Mbit/s down, 0.75 Mbit/s up and 300ms Chrome CDP latency, without CPU shaping. Loopback UI is 1,200×900 without shaping. Chrome is 157.0.8103.52. There is no physical device, DNS, TLS or production proxy.

| Payload | Transport | Encoded / decoded body B | Native HTTP median ms | Local UI median ms | Mobile UI median ms |
| --- | --- | ---: | ---: | ---: | ---: |
| zero00 | identity | 397 / 397 | 0.114 | 30.5 | 331.0 |
| zero00 | fastest | 310 / 397 | 0.130 | 31.0 | 331.1 |
| zero00 | default | 304 / 397 | 0.133 | 30.6 | 340.1 |
| small0 | identity | 1500 / 1500 | 0.109 | 27.8 | 342.0 |
| small0 | fastest | 599 / 1500 | 0.129 | 29.7 | 337.1 |
| small0 | default | 562 / 1500 | 0.139 | 30.4 | 343.0 |
| repeat | identity | 88390 / 88390 | 0.211 | 30.4 | 780.4 |
| repeat | fastest | 1860 / 88390 | 0.220 | 30.1 | 346.9 |
| repeat | default | 1131 / 88390 | 0.298 | 30.4 | 347.1 |
| random | identity | 88390 / 88390 | 0.224 | 29.5 | 780.8 |
| random | fastest | 52353 / 88390 | 0.477 | 29.1 | 596.7 |
| random | default | 51000 / 88390 | 1.488 | 29.8 | 596.9 |
| large0 | identity | 1105854 / 1105854 | 0.990 | 29.2 | 5887.1 |
| large0 | fastest | 12993 / 1105854 | 0.973 | 29.3 | 415.8 |
| large0 | default | 5994 / 1105854 | 2.214 | 29.7 | 388.9 |

These body sizes exclude HTTP/TCP overhead. All raw intervals, ranges and clock distributions remain in JSON. Local frame samples are similar or sometimes worse; small differences do not prove a universal latency improvement. The matrix uses the earlier default 32-byte floor. Its 397-byte zero00 control is below the final 768-byte floor and is prototype-only. Exact final-policy latency distributions remain open.

## Selection and small-response cutoff

Fastest was selected to limit per-request work while retaining most impaired-link gains. Default produced fewer bytes and was about 27ms faster on the largest repetitive UI fixture, but its native HTTP median was 2.214ms versus 0.973ms for Fastest. On varied ASCII, Default cost 1.488ms versus 0.477ms. This is a tradeoff, not proof that one quality is fastest for every workload.

A final finite-surface check found a small guest proposal that grew from 156 decoded body bytes to 160 gzip body bytes. Calibration then compared actual guest-proposal responses on an archived identity router and the applied Fastest router. Separate runtimes generate different IDs; matched raw JSON lengths and text are asserted. Serial socket bytesRead deltas count response HTTP headers, chunk framing and encoded body, excluding IP/TCP headers and TLS. Each size has four text styles (repeated, printable ASCII, Unicode and escaped characters) with three paired trials.

| Raw response B | Response TCP payload bytes saved, observed range | Non-saving pairs / 12 |
| ---: | ---: | ---: |
| 160 | -65 to -28 | 12 / 12 |
| 256 | -61 to 67 | 6 / 12 |
| 384 | -40 to 193 | 3 / 12 |
| 512 | -20 to 318 | 3 / 12 |
| 768 | 29 to 570 | 0 / 12 |
| 1024 | 72 to 825 | 0 / 12 |

768 bytes was the first tested size with positive traffic savings for every tested style/repetition. The final policy uses that conservative floor. Seventy-two paired final-policy controls confirm identical response TCP payload sizes below it and positive savings for the tested larger responses. Some smaller repetitive bodies would shrink, so this policy is not a universal entropy threshold or proof for every future payload/link.

## Verification and source scope

The prototype passes 131 browser/native checks, including decoding/hash parity, negotiation, owner/guest access, denial/expiry, draft/focus/selection preservation, 390px layout, changed events, genuine 15-second keepalives without snapshot GETs and revoked-stream reader release. The final working-tree router and server manifest files compile when combined with the archived backend and pass 49 checks plus the 72 byte controls. They do not validate the changing root backend or whole application.

The original browser run failed on inherited favicon 404s in all three variants. Its source, binary and failed report are retained. The real task router now answers the optional favicon request with an empty 204 response; this avoids another download of the existing 45,969-byte PWA PNG. No console error was filtered to obtain a pass. The PWA image route remains available. The initial random-data generator was corrected before measurement because its messages shared shifted sequences; the unmeasured earlier build remains recorded.

All experiment, calibration and final verification browsers/streams/native children are terminal with native exit 0. The measured matrix source/binary remains archived separately from later adoption builds. Full current-root integration is pending while the whiteboard owner integrates its modules and AppView::Whiteboards. Whiteboard-owned errors and files are left to that owner. Existing user-owned reactive/mobile authority fixes were not changed.

## Remaining scope

- Run the full current-root integration gate after the whiteboard owner completes that work. Leave whiteboard-owned transient errors and files to that owner.
- Measure exact final-policy UI/transport across current backend, physical devices, TLS/proxies, provider activity, concurrency and remaining task variants. The matrix uses the older backend and earlier 32-byte minimum.
- Review the conservative 768-byte floor on broader real payloads. Some smaller repetitive replies would shrink; a tested threshold is not a theorem for every payload or link.
- Complete semantic action review and equivalent workloads for all seven T3 comparisons. Source declarations, timing groups and functional checks are not whole-action acceptance.

The broader goal stays active. This study establishes no whole-action acceptance or verified faster-than-T3 comparison.

## Evidence

- [performance-task-compression3-timing1.json](target/performance-task-compression3-timing1.json)
- [performance-task-compression3-run-receipt.json](target/performance-task-compression3-run-receipt.json)
- [performance-task-compression3-observations.json](target/performance-task-compression3-observations.json)
- [performance-task-compression3-application-receipt.json](target/performance-task-compression3-application-receipt.json)
- [performance-task-compression3-verify01-readback.json](target/performance-task-compression3-verify01-readback.json)
- [performance-task-compression4-curve1.json](target/performance-task-compression4-curve1.json)
- [performance-task-compression5-boundary1.json](target/performance-task-compression5-boundary1.json)
- [performance-task-compression5-verify01-readback.json](target/performance-task-compression5-verify01-readback.json)
- [performance-discovery8-receipt.json](target/performance-discovery8-receipt.json)
