# Release asset compression measurements

Release SPA assets use build-time gzip sidecars. In this fixed-service browser fixture, precompression reduced the throttled fresh-start median from 69.33 s to 24.05 s and the reload median from 65.77 s to 1.08 s. Local reloads cost about 14 ms more, and the web-resource ZIP model grows by 5.31 MB. These results do not establish a matched win against any of the seven T3 targets.

## Retained measurements

Each cell has three retained observations after one warmup. Times start at navigation time origin and end after the release WASM app mounts, the chat composer is visible, six startup HTTP reply bodies succeed, three production SSE handshakes arrive, fonts load, and the command menu opens through two animation frames. Browser launch is excluded.

| Scenario | Raw median | Runtime gzip median | Precompressed median |
| --- | ---: | ---: | ---: |
| Loopback, fresh browser | 223.6 ms | 547.5 ms | 235.7 ms |
| Loopback, reload | 108.3 ms | 125.3 ms | 122.3 ms |
| 1.6 Mbit/s, 300 ms, fresh browser | 69.33 s | 24.17 s | 24.05 s |
| 1.6 Mbit/s, 300 ms, reload | 65.77 s | 1.09 s | 1.08 s |

[Raw retained samples, ranges and method](performance-static-assets-2026-11-13.json) keep every cell. Sample p95/p99 equal the maximum with three observations; they do not estimate population tails. The local fresh-start ranges overlap: 222.6–225.1 ms raw and 216.6–239.4 ms precompressed.

Compressed WASM came from the browser cache on reload; raw WASM transferred again. The WASM representation shrinks from 12,871,334 to 4,038,591 bytes. Representation size is not a transfer count on cache hits. CDP transferred-byte observations include response headers. Open SSE bytes are excluded; the separate aggregate of completed HTTP responses extends through post-boundary validation and is not total traffic through the UI timer.

## Method and tradeoffs

The study used Chrome 157.0.8103.52 on macOS arm64, Apple M2 Max, 32 GiB, at 1280×800. Six separate functional intervals passed at 390×844; this is not physical mobile testing. One warmup block and three retained blocks rotated strategy order and alternated link order. Each strategy/link pair launched a fresh browser process; reload reused that context. The OS filesystem was warm. Other user/system activity and three read-only source reviewers were present; no owned build, test, or other benchmark overlapped the timed study.

The ordinary optimized release SPA was frozen from the PR5 source tree now at main `0c140e8db1fef716c4ebb10276d91cbe6c322ae5`. The retained cohort used a prior gate's release assets with one stale, unreferenced 2,318-byte Markdown helper and its module preload. The final HTML omits it and reorders two remaining preloads. Final gated WASM and generated JavaScript binding bytes match the measured cohort exactly. Every strategy shared the same frozen inputs; the table is a controlled serving-strategy comparison, not exact final whole-action timings. A private test-only production-router fixture supplied fixed empty external-service replies and used real in-memory terminal and combined-SSE handlers. The browser remapped the baked API base to its private origin. No personal provider, capture, or filesystem endpoint ran. All 48 intervals succeeded, including 36 retained intervals. Requests already started at the clock boundary were validated afterward; that extra wait is excluded from the timer.

The impaired link used CDP at 1.6 Mbit/s download, 0.8 Mbit/s upload and 300 ms latency. Real phone CPU, radio behavior, packet loss, DNS/TLS, physical cold disk, populated histories, provider execution and multi-environment reconnect catch-up remain outside this study. T3's 0.7 MB target counts socket bootstrap metadata, and its reconnect target is not a page reload.

Runtime gzip avoids sidecar packaging but increased local fresh-start latency. Precompression uses build CPU and keeps serving cheap. All 131 regenerated sidecars have the same encoded lengths and compressed payloads as the measured prototype; the deterministic candidate uses gzip OS metadata 255 instead of 19. Both decompress to the exact originals.

The fixed ZIP-deflate6 model of final gate-generated web resources grows from 6,383,604 to 11,693,629 bytes with sidecars. This is a resource-only model, not a full signed app/update archive or an update-latency measurement. The 14 ms local reload cost and added package bytes are explicit tradeoffs for the impaired-link gains; this study does not prove that every action/client is faster.

## Build and compatibility

Trunk's release post-build hook atomically writes smaller deterministic gzip sidecars while preserving originals, modes and modification times. Debug builds skip compression. Python 3.9+ is required for the hook. The stage runs after asset pipelines and finalized staging HTML, per the [pinned Trunk pipeline](https://github.com/trunk-rs/trunk/blob/v0.21.14/src/pipelines/html.rs).

The companion server negotiates gzip only on its static fallback, retaining original MIME types, identity fallback and `Vary: Accept-Encoding` alongside CORS negotiation. API and SSE replies bypass this service. Tests cover gzip rejection, missing sidecars, HEAD metadata, byte ranges, conditional 304s, missing assets and live API/SSE handlers. Five build-hook tests cover artifact round trips, deterministic output, metadata, replacement/removal, debug/release behavior and missing staging directories.

Validation: the full 11-step merge gate passed (2,331 Rust tests, zero failures, six existing ignored tests; five Python build tests). Four functional browser flows passed with the actual gate-generated release assets at 1280×800 and 390×844. The native production router contract passed in the gate; the generated-asset UI flows used the same fixed-service fixture as the strategy study. All 130 final release sidecars decompress to exact originals.

Local full receipts remain under ignored `target/performance-network-boot1-*`, including `protocol.json`, `browser-run.json`, `timings.json`, `artifact-compatibility.json` and `code-review.json`. The committed JSON contains retained samples and receipt hashes. Failed warmup/probe attempts remain separate; no retained sample was discarded or relabeled. The broad action audit and final all-action timing log remain in progress.
