# Action timing log (incomplete)

The performance goal is active. This log lists discovered entrypoints and bounded measurements; it does not establish that every semantic action is catalogued or fast. The final version must include each supported action and its verified timings.

The catalogue has **354 source entrypoints** plus 1305 binding search references awaiting semantic review. There are 229 partial evidence records and 2004 normalized timing observations from bounded studies, plus 34 separate request-count cases without latency measurements. Whole-action acceptances and matched faster-than-T3 results are both zero. Older records remain linked while their numerical timings are normalized.

Normalized observations are typed timing groups. Different clocks can reuse the same underlying intervals, so this total is not a count of independent trials.

The 45 reviewed command intents retain their earlier source profiles: published main `53c1f1a3` and local working-tree base `c543e865`. New source discovery does not revalidate that semantic review. Timing rows keep their own captured build hashes. Concurrent edits after each capture are outside its snapshot. Local main contains concurrent unpublished work and has not integrated the latest published performance merge.

The [newer source-pinned discovery capture](target/performance-discovery8-receipt.json) includes 354 entrypoints from 458 files: 13 delivery routes, 3 local remote-host routes and 8 routes on the optional paired-task listener. Listener identities remain distinct. The isolated replay reproduces discovery fields; historical availability metadata stays separate. Exact source bytes are retained, but this text scan is not an atomic build snapshot, complete semantic action inventory or network acceptance. Whiteboards appears as a source declaration while its owner integrates the feature. Whiteboard-owned transient errors are left to that owner; no performance or full-workspace acceptance is inferred.

## T3 comparison targets

The screenshot mixes server reads, formatter CPU, bootstrap bytes and impaired-link completion. Each target keeps its original boundary. Source inspection identifies structural differences; it does not establish a timing win.

| Action | T3 reported | Bruhs finding | Required measurement |
| --- | ---: | --- | --- |
| Open a 41,000-item thread | [31 ms · first server read](https://github.com/pingdotgg/t3code/pull/34490) | Shipped hybrid: converted local load and recovery takes 55.15 ms versus 127.15 ms on the same 460-message / 41,000-part baseline fixture; untouched legacy read takes 123.15 ms. Full UI/paint and exact T3 data/boundary acceptance remain unverified. A separate full browser opening study measures 231.85 ms baseline versus 239.85 ms for an unadopted row-index candidate; this is not the T3 server read boundary. | Match mixed items, payload bytes, active work and visible-window semantics; measure server/store and complete UI opening separately, cold and warm. |
| Load earlier turns in a 45,000-item thread | [0.7 s · cold page read](https://github.com/pingdotgg/t3code/pull/34573) | Shipped hybrid: converted whole-history local load and recovery takes 65.15 ms versus 162.45 ms for 45,000 messages. This is not an earlier-turn range load; pagination and exact T3 acceptance remain open. | Bounded history read with preserved cursor/order/folding/search semantics, measured at newest/middle/oldest pages plus fallbacks. |
| Send to a 75,000-item thread | [90 ms · warm history read](https://github.com/pingdotgg/t3code/pull/35130) | Shipped hybrid: converted 75,000-message local append commit takes 199.2 ms versus 557.0 ms; first legacy append takes 346.35 ms versus 571.35 ms. Expected/draft setup and provider/network work are excluded; this is not T3 warm server-history reading. | Measure matched preparation/store boundary and real action-to-ack separately with provider work controlled; preserve all thread and queued-message semantics. |
| Launch mobile with 11 environments | [0.7 MB · bootstrap socket data](https://github.com/pingdotgg/t3code/pull/35072) | The inspected web client uses a build-time configured API base and responsive web UI. No equivalent 11-environment mobile launch measurement or feature contract has been established. Omitting environments/provider catalogs would not establish equivalent performance. A separate fixed-empty-service SPA asset study reduced a 12.87 MB WASM representation to 4.04 MB and measured impaired-link startup. It does not count 11-environment socket bootstrap metadata and closes no equivalent-client launch comparison. | Establish equivalent supported environment/client semantics, then count all bootstrap bytes and time to current usable state. |
| Reconnect mobile on slow 3G | [2.2 s · thread list current](https://github.com/pingdotgg/t3code/pull/32287) | Current loopback SSE/HTTP diagnostics do not cover this mobile cache/resume and over-cap snapshot workload. A responsive viewport check is also a different boundary. A fixed-empty-service page-reload study measured warm compressed-asset cache reuse on an emulated 1.6 Mbit/s / 300 ms link. Page reload does not verify disconnected multi-environment catch-up/replay semantics. | Match cache age/cursor gaps, compression and data volume; measure small-gap, over-cap, first-load and failed-resume paths through the impaired link. |
| Attach a terminal on a slow link | [751 ms · snapshot applied](https://github.com/pingdotgg/t3code/pull/33942) | Bruhs retains 512 KiB. Remote compression merged in PR#4 improves synthetic attach at 5 Mbps/120 ms round trip: 973.60 to 765.50 ms for entropy, 971.45 to 150.10 ms for repetitive logs, with six retained samples per case and exact byte/captured-state parity. Loopback stays raw at about 9–10 ms. Different data, OS and client boundary mean no verified T3 win. | Equivalent upstream fixture and functional boundary; real remote attach/reconnect/resync, more terminal controls/platforms, and full paint/mobile checks. |
| Render a long reasoning preview | [1.3 ms · formatter only](https://github.com/pingdotgg/t3code/pull/30038) | Collapsed web/native thinking rows render a fixed label and only materialize the reasoning body when expanded. This avoids the same full-Markdown formatter path but is not an equivalent textual preview, so it cannot count as a faster equivalent result. Message/storage cloning still needs checking. | Match displayed preview/full-expanded text and input distribution; measure formatter separately from collapsed/expanded actual client rendering and streaming updates. |

## Current bounded network timings

GET /api/meetings/stream, synthetic 16-event batches in 13-byte producer fragments. Each row is a separate process/group with 256 retained samples; p99 is its third-largest observation. These include two loopback TCP hops and exclude physical UI, real capture and remote DNS/TLS. They remain partial network variants.

`cursor` means compaction/borrowed frames only; `scan` means forward scanning only; `combined` includes both. Whole-batch input sends all event frames as one producer chunk; fragmented input uses at most 13 bytes per producer chunk. Both complete-delivery cases time receipt/framing of the final expected event; actual TCP chunk boundaries are not asserted.

| Variant | Group | p50 ms | p95 ms | p99 ms | Raw evidence |
| --- | --- | ---: | ---: | ---: | --- |
| baseline | a | 0.235042 | 0.269792 | 0.297042 | [sse-factorial-get--api-meetings-stream-2026-11-13](target/performance-sse-frames5-main-comparison.json) |
| cursor | a | 0.220083 | 0.268250 | 0.290666 | [sse-factorial-get--api-meetings-stream-2026-11-13](target/performance-sse-frames5-main-comparison.json) |
| combined | a | 0.231375 | 0.293208 | 0.517333 | [sse-factorial-get--api-meetings-stream-2026-11-13](target/performance-sse-frames5-main-comparison.json) |
| scan | a | 0.225375 | 0.274042 | 0.314250 | [sse-factorial-get--api-meetings-stream-2026-11-13](target/performance-sse-frames5-main-comparison.json) |
| cursor | b | 0.235291 | 0.290041 | 0.321375 | [sse-factorial-get--api-meetings-stream-2026-11-13](target/performance-sse-frames5-main-comparison.json) |
| scan | b | 0.239458 | 0.306666 | 0.361375 | [sse-factorial-get--api-meetings-stream-2026-11-13](target/performance-sse-frames5-main-comparison.json) |
| baseline | b | 0.240542 | 0.280166 | 0.332917 | [sse-factorial-get--api-meetings-stream-2026-11-13](target/performance-sse-frames5-main-comparison.json) |
| combined | b | 0.230333 | 0.276709 | 0.348292 | [sse-factorial-get--api-meetings-stream-2026-11-13](target/performance-sse-frames5-main-comparison.json) |
| scan | c | 0.230083 | 0.293666 | 0.355625 | [sse-factorial-get--api-meetings-stream-2026-11-13](target/performance-sse-frames5-main-comparison.json) |
| combined | c | 0.230625 | 0.282000 | 0.339042 | [sse-factorial-get--api-meetings-stream-2026-11-13](target/performance-sse-frames5-main-comparison.json) |
| cursor | c | 0.229875 | 0.294709 | 0.324333 | [sse-factorial-get--api-meetings-stream-2026-11-13](target/performance-sse-frames5-main-comparison.json) |
| baseline | c | 0.235125 | 0.291834 | 0.326917 | [sse-factorial-get--api-meetings-stream-2026-11-13](target/performance-sse-frames5-main-comparison.json) |
| combined | d | 0.231125 | 0.280916 | 0.308417 | [sse-factorial-get--api-meetings-stream-2026-11-13](target/performance-sse-frames5-main-comparison.json) |
| baseline | d | 0.230709 | 0.275167 | 0.300958 | [sse-factorial-get--api-meetings-stream-2026-11-13](target/performance-sse-frames5-main-comparison.json) |
| scan | d | 0.234708 | 0.305833 | 0.329834 | [sse-factorial-get--api-meetings-stream-2026-11-13](target/performance-sse-frames5-main-comparison.json) |
| cursor | d | 0.241000 | 0.286958 | 0.352083 | [sse-factorial-get--api-meetings-stream-2026-11-13](target/performance-sse-frames5-main-comparison.json) |

## Thread opening trial

Row-index candidate not adopted. Actual sidebar input through normal IndexedDB history, final-row layout and two frames. Six retained samples per cell; source/payload/visible-row details are preserved in the JSON. No physical paint or matched T3 acceptance.

| History | Action | Implementation | Median ms | Range ms | Raw evidence |
| --- | --- | --- | ---: | ---: | --- |
| small-control | sidebar pointer open | baseline | 48.300 | 48.000–48.500 | [web-thread-open-row-index1-2026-11-13](target/performance-timeline-rows1-timings.json) |
| small-control | sidebar pointer open | candidate | 48.200 | 47.300–48.400 | [web-thread-open-row-index1-2026-11-13](target/performance-timeline-rows1-timings.json) |
| wide-thinking-2000 | sidebar pointer open | baseline | 48.250 | 47.700–48.600 | [web-thread-open-row-index1-2026-11-13](target/performance-timeline-rows1-timings.json) |
| wide-thinking-2000 | sidebar pointer open | candidate | 48.300 | 48.100–49.100 | [web-thread-open-row-index1-2026-11-13](target/performance-timeline-rows1-timings.json) |
| wide-unfinished-2000 | sidebar pointer open | baseline | 134.500 | 133.400–166.500 | [web-thread-open-row-index1-2026-11-13](target/performance-timeline-rows1-timings.json) |
| wide-unfinished-2000 | sidebar pointer open | candidate | 133.300 | 132.100–168.600 | [web-thread-open-row-index1-2026-11-13](target/performance-timeline-rows1-timings.json) |
| mixed-41k-parts | sidebar pointer open | baseline | 231.850 | 222.600–232.200 | [web-thread-open-row-index1-2026-11-13](target/performance-timeline-rows1-timings.json) |
| mixed-41k-parts | sidebar pointer open | candidate | 239.850 | 214.800–251.600 | [web-thread-open-row-index1-2026-11-13](target/performance-timeline-rows1-timings.json) |
| many-turns-2000 | sidebar pointer open | baseline | 184.450 | 165.800–218.400 | [web-thread-open-row-index1-2026-11-13](target/performance-timeline-rows1-timings.json) |
| many-turns-2000 | sidebar pointer open | candidate | 174.400 | 165.700–199.900 | [web-thread-open-row-index1-2026-11-13](target/performance-timeline-rows1-timings.json) |
| long-reasoning-16m | sidebar pointer open | baseline | 64.800 | 64.600–65.700 | [web-thread-open-row-index1-2026-11-13](target/performance-timeline-rows1-timings.json) |
| long-reasoning-16m | sidebar pointer open | candidate | 64.650 | 64.200–70.200 | [web-thread-open-row-index1-2026-11-13](target/performance-timeline-rows1-timings.json) |

## Pin and unpin through local commit

Default shortcut through real thread-store commit, final-row layout and two frames, with an unsent draft. Six retained samples per cell. Original message/composer nodes and focus are preserved in candidate runs; exact records, transcript and draft match. Partial client/action variants; all-action and T3 acceptance remain open.

| History | Action | Implementation | Median ms | Range ms | Raw evidence |
| --- | --- | --- | ---: | ---: | --- |
| small-control | pin | baseline | 56.200 | 55.100–59.500 | [web-pin-chat-mount1-2026-11-13](target/performance-chat-mount1-timings.json) |
| small-control | pin | candidate | 39.400 | 38.500–52.100 | [web-pin-chat-mount1-2026-11-13](target/performance-chat-mount1-timings.json) |
| small-control | unpin | baseline | 54.250 | 51.400–56.100 | [web-pin-chat-mount1-2026-11-13](target/performance-chat-mount1-timings.json) |
| small-control | unpin | candidate | 38.650 | 37.000–50.700 | [web-pin-chat-mount1-2026-11-13](target/performance-chat-mount1-timings.json) |
| mixed-41k-parts | pin | baseline | 146.200 | 136.900–151.400 | [web-pin-chat-mount1-2026-11-13](target/performance-chat-mount1-timings.json) |
| mixed-41k-parts | pin | candidate | 49.950 | 40.500–56.300 | [web-pin-chat-mount1-2026-11-13](target/performance-chat-mount1-timings.json) |
| mixed-41k-parts | unpin | baseline | 129.200 | 125.800–142.900 | [web-pin-chat-mount1-2026-11-13](target/performance-chat-mount1-timings.json) |
| mixed-41k-parts | unpin | candidate | 38.800 | 24.400–47.300 | [web-pin-chat-mount1-2026-11-13](target/performance-chat-mount1-timings.json) |

## Resume command UI timings

Actual mounted web UI with existing synthetic history offscreen in Inbox. Native headless browser-memory Clipboard API; no OS Clipboard or physical paint. Six retained samples after one warmup per implementation/case/trigger. Menu open/filter, injected state setup, storage/history loading and verification are outside clocks. Primary rows end after Clipboard acknowledgement and two animation frames; no-write rows end after two frames. Invocation rows end at native writeText call. Zero readings are below timer resolution, not zero work. CPU work falls, while observed two-frame completion remains similar. No whole-action or matched T3 acceptance.

| Case | Trigger | Implementation | Clock | Median ms | Range ms | Raw evidence |
| --- | --- | --- | --- | ---: | ---: | --- |
| empty-codex | pointer | baseline | Clipboard and two frames | 48.400 | 47.500–48.800 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| empty-codex | pointer | baseline | Input to Clipboard call | 0.100 | 0.000–0.200 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| empty-codex | pointer | candidate | Clipboard and two frames | 48.600 | 47.800–50.400 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| empty-codex | pointer | candidate | Input to Clipboard call | 0.100 | 0.000–0.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| empty-codex | enter | baseline | Clipboard and two frames | 43.700 | 40.900–50.000 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| empty-codex | enter | baseline | Input to Clipboard call | 0.050 | 0.000–0.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| empty-codex | enter | candidate | Clipboard and two frames | 48.200 | 35.700–50.000 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| empty-codex | enter | candidate | Input to Clipboard call | 0.000 | 0.000–0.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| empty-codex | shortcut | baseline | Clipboard and two frames | 22.600 | 14.300–30.400 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| empty-codex | shortcut | baseline | Input to Clipboard call | 0.100 | 0.000–0.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| empty-codex | shortcut | candidate | Clipboard and two frames | 18.500 | 10.500–30.300 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| empty-codex | shortcut | candidate | Input to Clipboard call | 0.100 | 0.000–0.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| small-codex | pointer | baseline | Clipboard and two frames | 49.050 | 48.300–50.900 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| small-codex | pointer | baseline | Input to Clipboard call | 0.050 | 0.000–0.200 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| small-codex | pointer | candidate | Clipboard and two frames | 48.350 | 45.800–49.000 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| small-codex | pointer | candidate | Input to Clipboard call | 0.000 | 0.000–0.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| small-codex | enter | baseline | Clipboard and two frames | 41.000 | 34.700–48.400 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| small-codex | enter | baseline | Input to Clipboard call | 0.050 | 0.000–0.200 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| small-codex | enter | candidate | Clipboard and two frames | 48.800 | 46.600–50.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| small-codex | enter | candidate | Input to Clipboard call | 0.000 | 0.000–0.000 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| small-codex | shortcut | baseline | Clipboard and two frames | 19.050 | 6.500–30.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| small-codex | shortcut | baseline | Input to Clipboard call | 0.100 | 0.000–0.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| small-codex | shortcut | candidate | Clipboard and two frames | 18.800 | 6.200–30.800 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| small-codex | shortcut | candidate | Input to Clipboard call | 0.000 | 0.000–0.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| many-75k-codex | pointer | baseline | Clipboard and two frames | 48.850 | 45.200–50.600 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| many-75k-codex | pointer | baseline | Input to Clipboard call | 1.950 | 1.700–4.900 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| many-75k-codex | pointer | candidate | Clipboard and two frames | 48.600 | 47.600–49.900 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| many-75k-codex | pointer | candidate | Input to Clipboard call | 0.000 | 0.000–0.200 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| many-75k-codex | enter | baseline | Clipboard and two frames | 40.150 | 33.600–49.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| many-75k-codex | enter | baseline | Input to Clipboard call | 1.850 | 1.700–2.700 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| many-75k-codex | enter | candidate | Clipboard and two frames | 48.100 | 37.700–48.800 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| many-75k-codex | enter | candidate | Input to Clipboard call | 0.100 | 0.000–0.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| many-75k-codex | shortcut | baseline | Clipboard and two frames | 23.350 | 14.900–31.700 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| many-75k-codex | shortcut | baseline | Input to Clipboard call | 1.700 | 1.600–1.700 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| many-75k-codex | shortcut | candidate | Clipboard and two frames | 20.950 | 10.000–31.900 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| many-75k-codex | shortcut | candidate | Input to Clipboard call | 0.000 | 0.000–0.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| reasoning-16m-claude | pointer | baseline | Clipboard and two frames | 48.950 | 48.400–50.000 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| reasoning-16m-claude | pointer | baseline | Input to Clipboard call | 0.400 | 0.300–0.400 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| reasoning-16m-claude | pointer | candidate | Clipboard and two frames | 48.850 | 46.700–51.000 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| reasoning-16m-claude | pointer | candidate | Input to Clipboard call | 0.000 | 0.000–0.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| reasoning-16m-claude | enter | baseline | Clipboard and two frames | 44.300 | 33.600–49.200 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| reasoning-16m-claude | enter | baseline | Input to Clipboard call | 0.400 | 0.300–0.500 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| reasoning-16m-claude | enter | candidate | Clipboard and two frames | 48.000 | 36.200–49.000 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| reasoning-16m-claude | enter | candidate | Input to Clipboard call | 0.050 | 0.000–0.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| reasoning-16m-claude | shortcut | baseline | Clipboard and two frames | 22.900 | 14.800–30.400 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| reasoning-16m-claude | shortcut | baseline | Input to Clipboard call | 0.300 | 0.200–0.400 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| reasoning-16m-claude | shortcut | candidate | Clipboard and two frames | 21.350 | 11.600–30.900 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| reasoning-16m-claude | shortcut | candidate | Input to Clipboard call | 0.050 | 0.000–0.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| tool-output-8m-claude | pointer | baseline | Clipboard and two frames | 48.500 | 45.900–49.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| tool-output-8m-claude | pointer | baseline | Input to Clipboard call | 0.200 | 0.100–0.900 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| tool-output-8m-claude | pointer | candidate | Clipboard and two frames | 49.200 | 48.400–50.300 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| tool-output-8m-claude | pointer | candidate | Input to Clipboard call | 0.000 | 0.000–0.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| tool-output-8m-claude | enter | baseline | Clipboard and two frames | 48.550 | 34.300–49.300 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| tool-output-8m-claude | enter | baseline | Input to Clipboard call | 0.200 | 0.100–0.300 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| tool-output-8m-claude | enter | candidate | Clipboard and two frames | 44.450 | 37.700–50.500 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| tool-output-8m-claude | enter | candidate | Input to Clipboard call | 0.000 | 0.000–0.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| tool-output-8m-claude | shortcut | baseline | Clipboard and two frames | 23.050 | 16.000–30.400 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| tool-output-8m-claude | shortcut | baseline | Input to Clipboard call | 0.200 | 0.100–0.200 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| tool-output-8m-claude | shortcut | candidate | Clipboard and two frames | 20.300 | 10.700–29.800 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| tool-output-8m-claude | shortcut | candidate | Input to Clipboard call | 0.000 | 0.000–0.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| no-session-75k | pointer | baseline | Two frames; no write | 32.250 | 31.600–33.000 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| no-session-75k | pointer | candidate | Two frames; no write | 31.900 | 28.300–32.300 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| no-session-75k | enter | baseline | Two frames; no write | 27.450 | 20.600–32.700 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| no-session-75k | enter | candidate | Two frames; no write | 30.950 | 18.200–33.000 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| no-session-75k | shortcut | baseline | Two frames; no write | 23.200 | 15.900–30.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| no-session-75k | shortcut | candidate | Two frames; no write | 20.050 | 9.800–31.400 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| project-fallback-75k | pointer | baseline | Clipboard and two frames | 48.800 | 46.000–49.600 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| project-fallback-75k | pointer | baseline | Input to Clipboard call | 1.600 | 1.600–5.300 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| project-fallback-75k | pointer | candidate | Clipboard and two frames | 49.200 | 46.100–51.200 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| project-fallback-75k | pointer | candidate | Input to Clipboard call | 0.050 | 0.000–0.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| project-fallback-75k | enter | baseline | Clipboard and two frames | 41.650 | 33.400–49.400 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| project-fallback-75k | enter | baseline | Input to Clipboard call | 1.700 | 1.600–1.700 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| project-fallback-75k | enter | candidate | Clipboard and two frames | 48.150 | 36.600–48.900 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| project-fallback-75k | enter | candidate | Input to Clipboard call | 0.000 | 0.000–0.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| project-fallback-75k | shortcut | baseline | Clipboard and two frames | 23.000 | 14.700–32.000 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| project-fallback-75k | shortcut | baseline | Input to Clipboard call | 1.550 | 1.500–1.600 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| project-fallback-75k | shortcut | candidate | Clipboard and two frames | 19.500 | 7.800–31.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| project-fallback-75k | shortcut | candidate | Input to Clipboard call | 0.000 | 0.000–0.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| shell-quoting-small | pointer | baseline | Clipboard and two frames | 49.000 | 45.500–49.800 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| shell-quoting-small | pointer | baseline | Input to Clipboard call | 0.000 | 0.000–0.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| shell-quoting-small | pointer | candidate | Clipboard and two frames | 49.150 | 48.700–50.200 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| shell-quoting-small | pointer | candidate | Input to Clipboard call | 0.100 | 0.000–0.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| shell-quoting-small | enter | baseline | Clipboard and two frames | 43.350 | 33.300–48.600 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| shell-quoting-small | enter | baseline | Input to Clipboard call | 0.000 | 0.000–0.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| shell-quoting-small | enter | candidate | Clipboard and two frames | 44.750 | 34.600–48.000 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| shell-quoting-small | enter | candidate | Input to Clipboard call | 0.000 | 0.000–0.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| shell-quoting-small | shortcut | baseline | Clipboard and two frames | 21.350 | 9.200–28.500 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| shell-quoting-small | shortcut | baseline | Input to Clipboard call | 0.050 | 0.000–0.500 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| shell-quoting-small | shortcut | candidate | Clipboard and two frames | 19.400 | 10.700–29.000 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| shell-quoting-small | shortcut | candidate | Input to Clipboard call | 0.100 | 0.000–0.300 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| no-active | pointer | baseline | Two frames; no write | 32.050 | 30.000–33.100 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| no-active | pointer | candidate | Two frames; no write | 31.600 | 30.200–34.300 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| no-active | enter | baseline | Two frames; no write | 28.200 | 26.000–32.900 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| no-active | enter | candidate | Two frames; no write | 30.650 | 18.200–33.600 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| no-active | shortcut | baseline | Two frames; no write | 20.050 | 11.600–29.900 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |
| no-active | shortcut | candidate | Two frames; no write | 17.500 | 2.900–31.800 | [web-resume-export-ui1-2026-11-13](target/performance-export-resume1-timings.json) |

The [semantic action catalogue](docs/performance-action-catalogue.md) starts with 45 intents from 86 web/desktop registered commands. It does not establish downstream parity or complete application coverage. Working-tree search references now include callbacks after inline tests and global desktop app/dock code; comments/tests/lifecycle candidates remain unclassified.


The JSON companion retains all normalized cases, per-process quantiles, raw pointers, units and measurement scopes. Component decode timings and 128-event delivery diagnostics remain separate from the complete-delivery network rows.

## Static asset startup timings

Frozen ordinary release SPA with fixed empty services, real in-memory terminal and combined SSE, and mounted chat/command menu in desktop Chrome 157. Three retained intervals per cell after one warmup. Clock includes six startup reply bodies, three SSE handshakes, fonts and two frames after opening the menu. Fresh browser profile then same-context reload; OS filesystem warm. CDP impaired link is 1.6 Mbit/s / 300 ms. Physical mobile, radio, DNS/TLS, packet loss, populated histories and multi-environment catch-up are excluded. No matched T3 acceptance.

| Link and cache | Serving strategy | Samples | Median ms | Range ms | Raw evidence |
| --- | --- | ---: | ---: | ---: | --- |
| loopback/fresh browser | raw | 3 | 223.600 | 222.600–225.100 | [web-static-assets-network-boot1-2026-11-13](target/performance-network-boot1-timings.json) |
| loopback/warm reload | raw | 3 | 108.300 | 105.500–110.000 | [web-static-assets-network-boot1-2026-11-13](target/performance-network-boot1-timings.json) |
| emulated1.6mbps300ms/fresh browser | raw | 3 | 69327.400 | 69311.300–69330.200 | [web-static-assets-network-boot1-2026-11-13](target/performance-network-boot1-timings.json) |
| emulated1.6mbps300ms/warm reload | raw | 3 | 65773.300 | 65770.400–65801.600 | [web-static-assets-network-boot1-2026-11-13](target/performance-network-boot1-timings.json) |
| loopback/fresh browser | dynamic | 3 | 547.500 | 531.400–552.300 | [web-static-assets-network-boot1-2026-11-13](target/performance-network-boot1-timings.json) |
| loopback/warm reload | dynamic | 3 | 125.300 | 122.400–125.800 | [web-static-assets-network-boot1-2026-11-13](target/performance-network-boot1-timings.json) |
| emulated1.6mbps300ms/fresh browser | dynamic | 3 | 24168.600 | 24164.300–24175.300 | [web-static-assets-network-boot1-2026-11-13](target/performance-network-boot1-timings.json) |
| emulated1.6mbps300ms/warm reload | dynamic | 3 | 1086.800 | 1074.500–1090.200 | [web-static-assets-network-boot1-2026-11-13](target/performance-network-boot1-timings.json) |
| loopback/fresh browser | precompressed | 3 | 235.700 | 216.600–239.400 | [web-static-assets-network-boot1-2026-11-13](target/performance-network-boot1-timings.json) |
| loopback/warm reload | precompressed | 3 | 122.300 | 121.600–124.500 | [web-static-assets-network-boot1-2026-11-13](target/performance-network-boot1-timings.json) |
| emulated1.6mbps300ms/fresh browser | precompressed | 3 | 24046.400 | 24031.500–24046.700 | [web-static-assets-network-boot1-2026-11-13](target/performance-network-boot1-timings.json) |
| emulated1.6mbps300ms/warm reload | precompressed | 3 | 1075.400 | 1072.300–1090.900 | [web-static-assets-network-boot1-2026-11-13](target/performance-network-boot1-timings.json) |

Precompression costs about 14 ms at the local reload median and adds 5.31 MB in a web-resource-only ZIP model. Runtime gzip increases the local fresh-start median. Warm compressed WASM used browser cache; raw WASM transferred again. Three samples do not establish population tails. The separate completed-response byte aggregate extends through post-boundary validation and excludes open SSE. These remain partial startup variants; the full/signed update and equivalent T3 workloads are unmeasured.

## Terminal attach timings

Fixed, unmounted 1000 by 24 xterm parser with warm assets, exact 512 KiB replay and captured-state parity checks. The impaired connection runs at 5 Mbps with 120 ms round trip; direct runs use loopback. Timers end at the final xterm write callback. Paint, real mobile hardware, remote TCP connection setup, DNS and TLS are excluded. These remain partial action measurements; sample ranges do not establish population tail latency. Superseded prototypes and failed checks remain linked in the evidence record.

| Connection | Data | Version | Samples | Median ms | Range ms | WebSocket payload bytes | Raw evidence |
| --- | --- | --- | ---: | ---: | ---: | ---: | --- |
| 5 Mbps / 120 ms | entropy | main2d1a | 6 | 973.600 | 971.400–978.300 | 524288 | [terminal-attach-network-2026-11-13](target/performance-terminal1-timings.json) |
| 5 Mbps / 120 ms | entropy | remote gzip-v1 | 6 | 765.500 | 762.200–770.000 | 392001 | [terminal-attach-network-2026-11-13](target/performance-terminal1-timings.json) |
| 5 Mbps / 120 ms | repetitive | remote gzip-v1 | 6 | 150.100 | 149.500–150.400 | 11672 | [terminal-attach-network-2026-11-13](target/performance-terminal1-timings.json) |
| 5 Mbps / 120 ms | repetitive | main2d1a | 6 | 971.450 | 970.900–977.400 | 524288 | [terminal-attach-network-2026-11-13](target/performance-terminal1-timings.json) |
| loopback | entropy | main2d1a | 6 | 9.450 | 8.800–10.000 | 524288 | [terminal-attach-network-2026-11-13](target/performance-terminal1-timings.json) |
| loopback | entropy | automatic local raw | 6 | 9.700 | 8.900–11.000 | 524288 | [terminal-attach-network-2026-11-13](target/performance-terminal1-timings.json) |
| loopback | repetitive | automatic local raw | 6 | 9.400 | 9.100–9.900 | 524288 | [terminal-attach-network-2026-11-13](target/performance-terminal1-timings.json) |
| loopback | repetitive | main2d1a | 6 | 9.650 | 9.100–9.900 | 524288 | [terminal-attach-network-2026-11-13](target/performance-terminal1-timings.json) |

## Conversation storage timings

Preliminary actual-WASM measurements with synthetic IndexedDB histories, warm connections and three retained observations per row after one warmup. Browser version was not captured. Main application mounting, UI/paint and provider/remote network work are excluded. Load-and-prepare ends after read-only loading, raw-original retention and display recovery. Append times only the conflict-checked local commit; setup, readback verification and restoration are outside that timer. This is not T3’s warm server-history read. Input byte counts describe the JSON representation, not physical disk or network bytes.

| Synthetic history | Operation | Samples | Median ms | Range ms | Input JSON bytes | Raw evidence |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| small-control | load-prepare | 3 | 0.600 | 0.600–0.600 | 42465 | [web-history-storage-probe-2026-11-13](target/performance-history1-timings.json) |
| small-control | append | 3 | 1.200 | 1.200–1.400 | 42465 | [web-history-storage-probe-2026-11-13](target/performance-history1-timings.json) |
| mixed-41k-parts | load-prepare | 3 | 168.500 | 158.600–177.100 | 13799909 | [web-history-storage-probe-2026-11-13](target/performance-history1-timings.json) |
| mixed-41k-parts | append | 3 | 330.200 | 325.800–336.000 | 13799909 | [web-history-storage-probe-2026-11-13](target/performance-history1-timings.json) |
| messages-45k | load-prepare | 3 | 170.000 | 168.100–180.200 | 19471589 | [web-history-storage-probe-2026-11-13](target/performance-history1-timings.json) |
| messages-45k | append | 3 | 352.500 | 349.100–358.000 | 19471589 | [web-history-storage-probe-2026-11-13](target/performance-history1-timings.json) |
| messages-75k | load-prepare | 3 | 303.400 | 296.400–309.200 | 32474804 | [web-history-storage-probe-2026-11-13](target/performance-history1-timings.json) |
| messages-75k | append | 3 | 612.500 | 609.000–616.000 | 32474804 | [web-history-storage-probe-2026-11-13](target/performance-history1-timings.json) |
| large-tool-output | load-prepare | 3 | 89.700 | 88.500–91.200 | 16622135 | [web-history-storage-probe-2026-11-13](target/performance-history1-timings.json) |
| large-tool-output | append | 3 | 112.700 | 111.500–113.700 | 16622135 | [web-history-storage-probe-2026-11-13](target/performance-history1-timings.json) |

The JSON companion keeps codec component probes and correlated load/recovery stages separately. Zero stage readings are below the browser clock resolution, not zero work. The batch codec probe is not adopted: its direction varies by workload and operation. Full-record reads and writes still need separate bounded/incremental storage work.

## Conversation format comparisons

Warm actual-WASM storage measurements in private headless Chrome 157.0.8103.52. Each row has six retained intervals after one warmup, with counterbalanced implementation and case order. The full JSON envelope was rejected for large-output regressions; the hybrid format keeps small JSON headers and structured content/tool inputs. Converted reads improve, while untouched legacy records and structured fallback keep their existing paths. These are full-history local storage boundaries, not UI/paint, mobile, physical cold-I/O, network, server-send or T3 acceptance. Input bytes describe fixture JSON, not disk or wire bytes. Canonical hashes cannot distinguish non-finite values; the fallback fixture also checks exact float bits and typed commit/readback.

| Synthetic history | Storage boundary | Implementation | Samples | Median ms | Range ms | Raw evidence |
| --- | --- | --- | ---: | ---: | ---: | --- |
| small-control | legacy-load-prepare | main7e structured | 6 | 0.550 | 0.400–0.700 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| small-control | legacy-load-prepare | full JSON envelope (rejected) | 6 | 0.600 | 0.500–0.700 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| small-control | legacy-first-append | main7e structured | 6 | 1.000 | 0.900–1.400 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| small-control | legacy-first-append | full JSON envelope (rejected) | 6 | 0.800 | 0.700–1.000 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| small-control | formatted-load-prepare | main7e structured | 6 | 0.400 | 0.300–0.600 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| small-control | formatted-load-prepare | full JSON envelope (rejected) | 6 | 0.300 | 0.200–0.400 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| small-control | formatted-append | main7e structured | 6 | 1.000 | 0.900–1.100 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| small-control | formatted-append | full JSON envelope (rejected) | 6 | 0.650 | 0.500–0.800 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| mixed-41k-parts | legacy-load-prepare | main7e structured | 6 | 127.200 | 124.600–131.200 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| mixed-41k-parts | legacy-load-prepare | full JSON envelope (rejected) | 6 | 129.600 | 126.900–136.500 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| mixed-41k-parts | legacy-first-append | main7e structured | 6 | 282.250 | 278.100–286.000 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| mixed-41k-parts | legacy-first-append | full JSON envelope (rejected) | 6 | 192.800 | 187.600–195.400 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| mixed-41k-parts | formatted-load-prepare | main7e structured | 6 | 126.750 | 124.000–129.200 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| mixed-41k-parts | formatted-load-prepare | full JSON envelope (rejected) | 6 | 60.800 | 59.200–65.700 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| mixed-41k-parts | formatted-append | main7e structured | 6 | 276.350 | 274.600–288.400 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| mixed-41k-parts | formatted-append | full JSON envelope (rejected) | 6 | 125.600 | 123.600–130.600 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| messages-45k | legacy-load-prepare | main7e structured | 6 | 154.300 | 146.900–155.400 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| messages-45k | legacy-load-prepare | full JSON envelope (rejected) | 6 | 153.100 | 149.300–156.000 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| messages-45k | legacy-first-append | main7e structured | 6 | 322.250 | 316.100–324.100 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| messages-45k | legacy-first-append | full JSON envelope (rejected) | 6 | 219.100 | 217.500–222.100 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| messages-45k | formatted-load-prepare | main7e structured | 6 | 154.100 | 148.800–161.100 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| messages-45k | formatted-load-prepare | full JSON envelope (rejected) | 6 | 82.300 | 79.200–83.400 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| messages-45k | formatted-append | main7e structured | 6 | 327.000 | 314.700–331.200 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| messages-45k | formatted-append | full JSON envelope (rejected) | 6 | 153.050 | 151.000–154.300 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| messages-75k | legacy-load-prepare | main7e structured | 6 | 259.050 | 255.200–262.300 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| messages-75k | legacy-load-prepare | full JSON envelope (rejected) | 6 | 262.550 | 251.100–268.600 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| messages-75k | legacy-first-append | main7e structured | 6 | 548.100 | 542.600–565.600 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| messages-75k | legacy-first-append | full JSON envelope (rejected) | 6 | 370.300 | 364.400–381.700 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| messages-75k | formatted-load-prepare | main7e structured | 6 | 275.900 | 257.400–281.900 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| messages-75k | formatted-load-prepare | full JSON envelope (rejected) | 6 | 138.750 | 134.900–141.400 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| messages-75k | formatted-append | main7e structured | 6 | 552.700 | 544.000–570.900 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| messages-75k | formatted-append | full JSON envelope (rejected) | 6 | 257.100 | 253.500–260.900 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| large-tool-output | legacy-load-prepare | main7e structured | 6 | 29.450 | 27.400–31.400 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| large-tool-output | legacy-load-prepare | full JSON envelope (rejected) | 6 | 28.500 | 27.800–39.800 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| large-tool-output | legacy-first-append | main7e structured | 6 | 50.100 | 48.600–52.300 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| large-tool-output | legacy-first-append | full JSON envelope (rejected) | 6 | 66.200 | 64.900–71.900 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| large-tool-output | formatted-load-prepare | main7e structured | 6 | 28.600 | 28.000–29.000 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| large-tool-output | formatted-load-prepare | full JSON envelope (rejected) | 6 | 39.250 | 38.900–40.000 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| large-tool-output | formatted-append | main7e structured | 6 | 51.100 | 49.300–51.600 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| large-tool-output | formatted-append | full JSON envelope (rejected) | 6 | 77.250 | 76.600–81.900 | [web-conversation-history2-envelope-2026-11-13](target/performance-history2-timings.json) |
| small-control | legacy-load-prepare | main7e structured | 6 | 0.500 | 0.400–0.600 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| small-control | legacy-load-prepare | JSON headers with structured content candidate | 6 | 0.600 | 0.500–0.700 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| small-control | legacy-first-append | main7e structured | 6 | 1.100 | 0.900–1.200 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| small-control | legacy-first-append | JSON headers with structured content candidate | 6 | 0.800 | 0.700–0.900 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| small-control | formatted-load-prepare | main7e structured | 6 | 0.500 | 0.400–0.500 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| small-control | formatted-load-prepare | JSON headers with structured content candidate | 6 | 0.250 | 0.200–0.300 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| small-control | formatted-append | main7e structured | 6 | 0.950 | 0.800–1.100 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| small-control | formatted-append | JSON headers with structured content candidate | 6 | 0.550 | 0.500–0.700 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| mixed-41k-parts | legacy-load-prepare | main7e structured | 6 | 126.550 | 122.600–130.200 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| mixed-41k-parts | legacy-load-prepare | JSON headers with structured content candidate | 6 | 123.150 | 119.900–126.600 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| mixed-41k-parts | legacy-first-append | main7e structured | 6 | 281.100 | 274.400–293.500 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| mixed-41k-parts | legacy-first-append | JSON headers with structured content candidate | 6 | 177.700 | 175.700–180.900 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| mixed-41k-parts | formatted-load-prepare | main7e structured | 6 | 127.150 | 125.700–128.800 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| mixed-41k-parts | formatted-load-prepare | JSON headers with structured content candidate | 6 | 55.150 | 55.000–58.300 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| mixed-41k-parts | formatted-append | main7e structured | 6 | 275.300 | 268.800–289.600 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| mixed-41k-parts | formatted-append | JSON headers with structured content candidate | 6 | 111.400 | 109.100–114.600 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| messages-45k | legacy-load-prepare | main7e structured | 6 | 158.000 | 153.300–161.800 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| messages-45k | legacy-load-prepare | JSON headers with structured content candidate | 6 | 152.300 | 149.600–153.800 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| messages-45k | legacy-first-append | main7e structured | 6 | 331.600 | 325.200–338.700 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| messages-45k | legacy-first-append | JSON headers with structured content candidate | 6 | 200.150 | 196.100–204.700 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| messages-45k | formatted-load-prepare | main7e structured | 6 | 162.450 | 150.500–164.900 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| messages-45k | formatted-load-prepare | JSON headers with structured content candidate | 6 | 65.150 | 60.500–69.600 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| messages-45k | formatted-append | main7e structured | 6 | 333.700 | 321.900–341.000 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| messages-45k | formatted-append | JSON headers with structured content candidate | 6 | 111.700 | 107.800–116.600 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| messages-75k | legacy-load-prepare | main7e structured | 6 | 264.500 | 257.100–269.400 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| messages-75k | legacy-load-prepare | JSON headers with structured content candidate | 6 | 261.800 | 256.600–263.100 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| messages-75k | legacy-first-append | main7e structured | 6 | 571.350 | 567.200–587.400 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| messages-75k | legacy-first-append | JSON headers with structured content candidate | 6 | 346.350 | 340.800–354.200 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| messages-75k | formatted-load-prepare | main7e structured | 6 | 282.100 | 258.200–296.400 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| messages-75k | formatted-load-prepare | JSON headers with structured content candidate | 6 | 115.300 | 105.300–125.900 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| messages-75k | formatted-append | main7e structured | 6 | 557.000 | 549.200–577.900 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| messages-75k | formatted-append | JSON headers with structured content candidate | 6 | 199.200 | 192.900–203.500 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| large-tool-output | legacy-load-prepare | main7e structured | 6 | 29.700 | 28.300–31.800 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| large-tool-output | legacy-load-prepare | JSON headers with structured content candidate | 6 | 28.850 | 27.500–30.000 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| large-tool-output | legacy-first-append | main7e structured | 6 | 51.300 | 50.200–54.600 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| large-tool-output | legacy-first-append | JSON headers with structured content candidate | 6 | 42.350 | 41.200–42.800 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| large-tool-output | formatted-load-prepare | main7e structured | 6 | 28.350 | 28.200–29.300 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| large-tool-output | formatted-load-prepare | JSON headers with structured content candidate | 6 | 15.600 | 14.800–16.100 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| large-tool-output | formatted-append | main7e structured | 6 | 51.900 | 50.200–57.600 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| large-tool-output | formatted-append | JSON headers with structured content candidate | 6 | 27.950 | 24.700–29.200 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| long-reasoning | legacy-load-prepare | main7e structured | 6 | 5.100 | 4.800–5.600 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| long-reasoning | legacy-load-prepare | JSON headers with structured content candidate | 6 | 5.100 | 4.800–5.600 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| long-reasoning | legacy-first-append | main7e structured | 6 | 12.100 | 11.500–14.400 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| long-reasoning | legacy-first-append | JSON headers with structured content candidate | 6 | 8.500 | 8.200–9.000 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| long-reasoning | formatted-load-prepare | main7e structured | 6 | 5.050 | 5.000–5.200 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| long-reasoning | formatted-load-prepare | JSON headers with structured content candidate | 6 | 2.550 | 2.500–2.700 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| long-reasoning | formatted-append | main7e structured | 6 | 11.600 | 11.100–12.000 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| long-reasoning | formatted-append | JSON headers with structured content candidate | 6 | 5.950 | 5.800–6.900 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| escaped-output | legacy-load-prepare | main7e structured | 6 | 11.450 | 11.200–11.900 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| escaped-output | legacy-load-prepare | JSON headers with structured content candidate | 6 | 11.800 | 11.100–12.700 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| escaped-output | legacy-first-append | main7e structured | 6 | 27.850 | 26.200–34.900 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| escaped-output | legacy-first-append | JSON headers with structured content candidate | 6 | 20.150 | 18.700–20.500 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| escaped-output | formatted-load-prepare | main7e structured | 6 | 11.300 | 11.200–12.200 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| escaped-output | formatted-load-prepare | JSON headers with structured content candidate | 6 | 5.400 | 5.300–6.400 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| escaped-output | formatted-append | main7e structured | 6 | 26.100 | 25.700–27.300 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| escaped-output | formatted-append | JSON headers with structured content candidate | 6 | 12.450 | 12.100–12.900 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| structured-fallback | legacy-load-prepare | main7e structured | 6 | 0.600 | 0.500–0.700 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| structured-fallback | legacy-load-prepare | JSON headers with structured content candidate | 6 | 0.500 | 0.400–0.700 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| structured-fallback | legacy-first-append | main7e structured | 6 | 1.100 | 1.000–1.200 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| structured-fallback | legacy-first-append | JSON headers with structured content candidate | 6 | 1.100 | 1.000–1.200 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| structured-fallback | formatted-load-prepare | main7e structured | 6 | 0.500 | 0.400–0.500 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| structured-fallback | formatted-load-prepare | JSON headers with structured content candidate | 6 | 0.500 | 0.400–0.600 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| structured-fallback | formatted-append | main7e structured | 6 | 1.050 | 0.900–1.100 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |
| structured-fallback | formatted-append | JSON headers with structured content candidate | 6 | 1.050 | 1.000–1.200 | [web-conversation-history3-envelope-2026-11-13](target/performance-history3-timings.json) |

A formatted row means preparation through the actual writer; the fallback fixture remains structured. First legacy append includes conversion in the checked save. Setup, readback/restoration and wrapper diagnostics are outside the append timer. Small control/legacy/fallback differences overlap clock resolution or observed variation. Six samples do not establish population tails. Bounded history reads, incremental writes, full UI flows and the broader network/action audit remain required.

## Conversation save-queue completion

Actual optimized WASM TaskSaves edit through original-checked IndexedDB commit, acknowledgement and queue drain, including existing scheduler yields. The app is unmounted. Load/recovery, caller draft construction, verification, hashing and restoration are outside clocks. Six retained samples per cell after one warmup; case, format and implementation order counterbalanced. Prepared rows use the actual writer; non-finite fallback remains structured. Larger histories improve while short-history completion is mixed. Zero enqueue readings are below timer resolution. No whole-action, UI/paint, network/provider or matched T3 acceptance.

| Synthetic history | Format | Version | Median ms | Range ms | Raw evidence |
| --- | --- | --- | ---: | ---: | --- |
| small-control | legacy | baseline | 18.450 | 2.500–29.000 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| small-control | legacy | candidate | 19.050 | 1.600–35.800 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| small-control | formatted | baseline | 8.200 | 0.800–17.900 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| small-control | formatted | candidate | 15.550 | 2.000–32.800 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| mixed-41k-parts | legacy | baseline | 216.300 | 212.500–219.900 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| mixed-41k-parts | legacy | candidate | 202.750 | 197.900–211.200 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| mixed-41k-parts | formatted | baseline | 140.000 | 135.900–143.800 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| mixed-41k-parts | formatted | candidate | 121.750 | 115.500–135.100 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| messages-45k | legacy | baseline | 247.900 | 236.900–260.300 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| messages-45k | legacy | candidate | 235.900 | 227.300–251.700 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| messages-45k | formatted | baseline | 155.600 | 143.300–168.900 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| messages-45k | formatted | candidate | 136.700 | 125.300–148.900 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| messages-75k | legacy | baseline | 410.550 | 392.700–416.200 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| messages-75k | legacy | candidate | 395.550 | 383.200–408.900 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| messages-75k | formatted | baseline | 269.400 | 241.400–299.300 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| messages-75k | formatted | candidate | 238.700 | 228.600–246.500 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| large-tool-output | legacy | baseline | 48.450 | 47.400–49.700 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| large-tool-output | legacy | candidate | 47.950 | 44.100–48.900 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| large-tool-output | formatted | baseline | 44.000 | 40.400–49.600 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| large-tool-output | formatted | candidate | 45.050 | 32.200–52.200 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| long-reasoning | legacy | baseline | 11.850 | 10.900–33.900 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| long-reasoning | legacy | candidate | 11.500 | 9.200–14.900 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| long-reasoning | formatted | baseline | 9.200 | 7.100–17.300 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| long-reasoning | formatted | candidate | 11.450 | 7.100–19.000 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| escaped-output | legacy | baseline | 37.950 | 25.200–45.400 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| escaped-output | legacy | candidate | 25.450 | 20.400–42.200 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| escaped-output | formatted | baseline | 16.700 | 15.400–20.700 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| escaped-output | formatted | candidate | 16.000 | 14.100–18.300 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| structured-fallback | legacy | baseline | 17.100 | 7.400–28.600 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| structured-fallback | legacy | candidate | 15.200 | 1.900–32.500 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| structured-fallback | formatted | baseline | 13.550 | 1.600–30.700 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |
| structured-fallback | formatted | candidate | 18.450 | 10.300–28.100 | [web-conversation-snapshot1-2026-11-13](target/performance-snapshot1-timings.json) |

### Focused small-case replication

Same candidate and main binaries, with 32 retained observations per cell after one warmup. The candidate remains withheld: prepared structured fallback median completion is 9.8 ms on main and 26.3 ms on the candidate. Other short-history directions vary. The save loop yields again before noticing an empty queue; causation is unproven and requires a separate commit-boundary/scheduler investigation. All 396 intervals pass full readback, ordered counts and float checks.

| Synthetic history | Format | Version | Median ms | Range ms | Raw evidence |
| --- | --- | --- | ---: | ---: | --- |
| small-control | legacy | baseline | 27.550 | 10.000–29.900 | [web-conversation-snapshot1-small-control1-2026-11-13](target/performance-snapshot1-small-control1-timings.json) |
| small-control | legacy | candidate | 11.400 | 10.300–28.500 | [web-conversation-snapshot1-small-control1-2026-11-13](target/performance-snapshot1-small-control1-timings.json) |
| small-control | formatted | baseline | 10.100 | 7.100–27.400 | [web-conversation-snapshot1-small-control1-2026-11-13](target/performance-snapshot1-small-control1-timings.json) |
| small-control | formatted | candidate | 10.600 | 1.100–18.400 | [web-conversation-snapshot1-small-control1-2026-11-13](target/performance-snapshot1-small-control1-timings.json) |
| long-reasoning | legacy | baseline | 10.450 | 8.900–18.300 | [web-conversation-snapshot1-small-control1-2026-11-13](target/performance-snapshot1-small-control1-timings.json) |
| long-reasoning | legacy | candidate | 9.600 | 8.800–11.000 | [web-conversation-snapshot1-small-control1-2026-11-13](target/performance-snapshot1-small-control1-timings.json) |
| long-reasoning | formatted | baseline | 13.400 | 7.500–18.800 | [web-conversation-snapshot1-small-control1-2026-11-13](target/performance-snapshot1-small-control1-timings.json) |
| long-reasoning | formatted | candidate | 7.350 | 6.800–18.400 | [web-conversation-snapshot1-small-control1-2026-11-13](target/performance-snapshot1-small-control1-timings.json) |
| structured-fallback | legacy | baseline | 9.850 | 1.300–18.200 | [web-conversation-snapshot1-small-control1-2026-11-13](target/performance-snapshot1-small-control1-timings.json) |
| structured-fallback | legacy | candidate | 12.450 | 10.000–29.500 | [web-conversation-snapshot1-small-control1-2026-11-13](target/performance-snapshot1-small-control1-timings.json) |
| structured-fallback | formatted | baseline | 9.800 | 1.700–26.800 | [web-conversation-snapshot1-small-control1-2026-11-13](target/performance-snapshot1-small-control1-timings.json) |
| structured-fallback | formatted | candidate | 26.300 | 9.400–28.800 | [web-conversation-snapshot1-small-control1-2026-11-13](target/performance-snapshot1-small-control1-timings.json) |

## Save commit and worker completion

New release-WASM studies separate the earliest IndexedDB complete event from acknowledgement and worker drain. The early observer registers when the native transaction is created, before Rust handlers; a late observer and frame ordinals check ordering. Existing Rust enqueue/drain times must match the three-clock bridge exactly. These are unmounted private local-store variants; caller draft preparation, loading/recovery, verification, restoration, physical UI and network/provider work are excluded.

Primary rows below use six retained observations after one warmup, natural entry timing and writer-prepared records. Non-finite fallback remains structured. Each successful candidate save schedules one frame; main schedules two. The complete legacy variants and five-clock distributions remain in JSON. Sample ranges are not population tails.

| History | Endpoint | Version | Median ms | Range ms | Raw evidence |
| --- | --- | --- | ---: | ---: | --- |
| small-control | commit_ms | baseline | 1.500 | 0.800–3.400 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| small-control | drain_ms | baseline | 17.900 | 11.700–19.300 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| small-control | commit_ms | candidate | 1.650 | 1.000–6.100 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| small-control | drain_ms | candidate | 1.650 | 1.000–6.100 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| mixed-41k-parts | commit_ms | baseline | 143.950 | 119.500–154.500 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| mixed-41k-parts | drain_ms | baseline | 153.850 | 129.100–164.300 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| mixed-41k-parts | commit_ms | candidate | 107.350 | 106.700–120.000 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| mixed-41k-parts | drain_ms | candidate | 111.200 | 110.600–123.700 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| messages-45k | commit_ms | baseline | 135.750 | 131.500–151.800 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| messages-45k | drain_ms | baseline | 144.450 | 140.000–160.800 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| messages-45k | commit_ms | candidate | 128.400 | 122.500–133.400 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| messages-45k | drain_ms | candidate | 132.350 | 126.400–137.500 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| messages-75k | commit_ms | baseline | 243.650 | 236.500–255.300 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| messages-75k | drain_ms | baseline | 258.800 | 251.300–270.500 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| messages-75k | commit_ms | candidate | 218.350 | 207.400–238.900 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| messages-75k | drain_ms | candidate | 224.950 | 214.000–245.300 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| large-tool-output | commit_ms | baseline | 28.700 | 25.900–44.100 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| large-tool-output | drain_ms | baseline | 42.700 | 29.400–47.600 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| large-tool-output | commit_ms | candidate | 25.200 | 24.200–26.100 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| large-tool-output | drain_ms | candidate | 28.150 | 27.100–29.100 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| long-reasoning | commit_ms | baseline | 6.600 | 6.300–6.800 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| long-reasoning | drain_ms | baseline | 13.200 | 7.000–17.900 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| long-reasoning | commit_ms | candidate | 6.350 | 6.000–6.500 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| long-reasoning | drain_ms | candidate | 6.800 | 6.300–7.000 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| escaped-output | commit_ms | baseline | 13.700 | 13.500–15.100 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| escaped-output | drain_ms | baseline | 15.950 | 15.100–17.300 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| escaped-output | commit_ms | candidate | 12.950 | 12.900–14.100 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| escaped-output | drain_ms | candidate | 14.050 | 13.900–15.200 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| structured-fallback | commit_ms | baseline | 9.000 | 1.200–15.400 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| structured-fallback | drain_ms | baseline | 24.200 | 14.800–30.500 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| structured-fallback | commit_ms | candidate | 5.600 | 2.800–9.200 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |
| structured-fallback | drain_ms | candidate | 5.600 | 2.800–9.200 | [web-conversation-save2-2026-11-13](target/performance-save2-timings.json) |

### Balanced requested entry phases

The independent focused cohort retains 32 samples per cell, eight at each requested fraction of a calibrated frame. Actual scheduling still varies; requested phase balance does not guarantee identical delivery. Keep the unfavorable entry-to-commit cells visible. After-yield short writes remain similar; worker completion improves. Phase setup is outside clocks. All 620 comparison saves pass complete readback/count/float, exact-clock and frame checks; 40 untimed controls pass. This establishes no whole-action or T3 acceptance.

| History | Endpoint | Version | Median ms | Range ms | Raw evidence |
| --- | --- | --- | ---: | ---: | --- |
| small-control | commit_ms | baseline | 8.350 | 0.600–18.400 | [web-conversation-save2-phase-control1-2026-11-13](target/performance-save2-phase-control1-timings.json) |
| small-control | drain_ms | baseline | 24.300 | 12.200–35.100 | [web-conversation-save2-phase-control1-2026-11-13](target/performance-save2-phase-control1-timings.json) |
| small-control | after_yield_commit_ms | baseline | 0.700 | 0.600–1.300 | [web-conversation-save2-phase-control1-2026-11-13](target/performance-save2-phase-control1-timings.json) |
| small-control | commit_ms | candidate | 8.750 | 0.700–19.000 | [web-conversation-save2-phase-control1-2026-11-13](target/performance-save2-phase-control1-timings.json) |
| small-control | drain_ms | candidate | 8.750 | 0.700–19.000 | [web-conversation-save2-phase-control1-2026-11-13](target/performance-save2-phase-control1-timings.json) |
| small-control | after_yield_commit_ms | candidate | 0.750 | 0.500–3.900 | [web-conversation-save2-phase-control1-2026-11-13](target/performance-save2-phase-control1-timings.json) |
| long-reasoning | commit_ms | baseline | 11.650 | 6.000–24.200 | [web-conversation-save2-phase-control1-2026-11-13](target/performance-save2-phase-control1-timings.json) |
| long-reasoning | drain_ms | baseline | 21.650 | 6.600–35.100 | [web-conversation-save2-phase-control1-2026-11-13](target/performance-save2-phase-control1-timings.json) |
| long-reasoning | after_yield_commit_ms | baseline | 6.100 | 5.800–6.600 | [web-conversation-save2-phase-control1-2026-11-13](target/performance-save2-phase-control1-timings.json) |
| long-reasoning | commit_ms | candidate | 11.900 | 6.100–19.300 | [web-conversation-save2-phase-control1-2026-11-13](target/performance-save2-phase-control1-timings.json) |
| long-reasoning | drain_ms | candidate | 12.300 | 6.500–19.700 | [web-conversation-save2-phase-control1-2026-11-13](target/performance-save2-phase-control1-timings.json) |
| long-reasoning | after_yield_commit_ms | candidate | 6.100 | 5.800–7.100 | [web-conversation-save2-phase-control1-2026-11-13](target/performance-save2-phase-control1-timings.json) |
| structured-fallback | commit_ms | baseline | 6.350 | 1.100–18.600 | [web-conversation-save2-phase-control1-2026-11-13](target/performance-save2-phase-control1-timings.json) |
| structured-fallback | drain_ms | baseline | 22.300 | 10.500–34.700 | [web-conversation-save2-phase-control1-2026-11-13](target/performance-save2-phase-control1-timings.json) |
| structured-fallback | after_yield_commit_ms | baseline | 1.100 | 0.900–1.400 | [web-conversation-save2-phase-control1-2026-11-13](target/performance-save2-phase-control1-timings.json) |
| structured-fallback | commit_ms | candidate | 12.200 | 1.300–19.700 | [web-conversation-save2-phase-control1-2026-11-13](target/performance-save2-phase-control1-timings.json) |
| structured-fallback | drain_ms | candidate | 12.200 | 1.300–19.700 | [web-conversation-save2-phase-control1-2026-11-13](target/performance-save2-phase-control1-timings.json) |
| structured-fallback | after_yield_commit_ms | candidate | 1.100 | 0.900–8.000 | [web-conversation-save2-phase-control1-2026-11-13](target/performance-save2-phase-control1-timings.json) |

The 204-interval diagnostic with the previous Arc-only candidate remains separate. The earlier late-listener experiment and capture-flag preflight are excluded as commit-boundary evidence. Source and observer corrections are retained with the study. Bounded history, actual UI/paint, provider/network and semantic all-action coverage remain open.


## Direct original-envelope comparison (withheld)

Published main (`53c1f1a3`) versus a strict direct-envelope check with typed fallback. Both versions keep one pre-write frame and the v12 format. The candidate is unmerged: long writer-prepared message lists have lower observed medians, while tool-heavy and legacy directions vary. Full histories are still read and rewritten. Six retained observations per cell after one warmup; 224 saves pass complete history/float/cache, exact-clock and event-order checks. Twenty-one codec controls and twenty functional saves pass. These unmounted local-store clocks exclude load/recovery/caller draft construction, verification/hash/restore, UI/paint, physical cold I/O, provider and network completion. Ranges are descriptive samples, not population tails.

| History | Endpoint | Version | Median ms | Range ms | Raw evidence |
| --- | --- | --- | ---: | ---: | --- |
| small-control | commit_ms | baseline | 2.550 | 0.800–4.200 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| small-control | drain_ms | baseline | 2.550 | 0.800–4.200 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| small-control | after_yield_commit_ms | baseline | 0.600 | 0.600–1.700 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| small-control | commit_ms | candidate | 1.400 | 0.500–17.100 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| small-control | drain_ms | candidate | 1.400 | 0.600–17.100 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| small-control | after_yield_commit_ms | candidate | 0.650 | 0.500–1.900 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| mixed-41k-parts | commit_ms | baseline | 113.050 | 108.300–132.800 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| mixed-41k-parts | drain_ms | baseline | 117.000 | 112.200–136.700 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| mixed-41k-parts | after_yield_commit_ms | baseline | 112.750 | 108.000–131.800 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| mixed-41k-parts | commit_ms | candidate | 114.500 | 102.700–138.800 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| mixed-41k-parts | drain_ms | candidate | 118.450 | 106.600–142.600 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| mixed-41k-parts | after_yield_commit_ms | candidate | 110.500 | 102.400–127.500 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| messages-45k | commit_ms | baseline | 127.750 | 117.900–145.000 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| messages-45k | drain_ms | baseline | 131.750 | 121.700–149.000 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| messages-45k | after_yield_commit_ms | baseline | 126.700 | 117.600–143.300 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| messages-45k | commit_ms | candidate | 115.350 | 111.300–119.200 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| messages-45k | drain_ms | candidate | 119.250 | 115.100–123.200 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| messages-45k | after_yield_commit_ms | candidate | 114.800 | 111.000–118.500 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| messages-75k | commit_ms | baseline | 215.650 | 208.400–243.200 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| messages-75k | drain_ms | baseline | 222.050 | 215.100–249.500 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| messages-75k | after_yield_commit_ms | baseline | 215.400 | 207.900–242.900 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| messages-75k | commit_ms | candidate | 201.050 | 189.300–214.700 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| messages-75k | drain_ms | candidate | 207.450 | 195.500–221.100 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| messages-75k | after_yield_commit_ms | candidate | 195.950 | 189.000–204.000 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| large-tool-output | commit_ms | baseline | 25.850 | 24.300–28.300 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| large-tool-output | drain_ms | baseline | 28.800 | 27.200–31.200 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| large-tool-output | after_yield_commit_ms | baseline | 25.650 | 24.100–28.000 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| large-tool-output | commit_ms | candidate | 26.950 | 24.000–39.600 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| large-tool-output | drain_ms | candidate | 30.000 | 26.900–42.700 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| large-tool-output | after_yield_commit_ms | candidate | 26.750 | 23.700–38.400 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| long-reasoning | commit_ms | baseline | 6.450 | 6.100–6.900 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| long-reasoning | drain_ms | baseline | 7.000 | 6.600–7.300 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| long-reasoning | after_yield_commit_ms | baseline | 6.250 | 5.900–6.700 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| long-reasoning | commit_ms | candidate | 6.400 | 6.100–7.500 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| long-reasoning | drain_ms | candidate | 6.900 | 6.600–7.900 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| long-reasoning | after_yield_commit_ms | candidate | 6.150 | 5.900–7.200 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| escaped-output | commit_ms | baseline | 12.750 | 12.200–14.000 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| escaped-output | drain_ms | baseline | 13.950 | 13.200–15.200 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| escaped-output | after_yield_commit_ms | baseline | 12.500 | 12.000–13.700 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| escaped-output | commit_ms | candidate | 13.300 | 12.600–17.400 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| escaped-output | drain_ms | candidate | 14.500 | 13.600–18.500 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| escaped-output | after_yield_commit_ms | candidate | 12.950 | 12.300–17.200 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| structured-fallback | commit_ms | baseline | 8.500 | 1.000–17.400 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| structured-fallback | drain_ms | baseline | 8.500 | 1.100–17.400 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| structured-fallback | after_yield_commit_ms | baseline | 1.050 | 0.900–1.200 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| structured-fallback | commit_ms | candidate | 1.550 | 0.900–8.800 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| structured-fallback | drain_ms | candidate | 1.550 | 1.000–8.900 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |
| structured-fallback | after_yield_commit_ms | candidate | 0.950 | 0.900–1.000 | [web-conversation-envelope3-2026-11-13](target/performance-envelope3-timings.json) |

The after_yield_commit_ms clock runs from the first pre-save scheduler callback through early transaction completion. commit_ms runs from edit_conversation entry through that same completion event. drain_ms runs from edit entry until the worker clears saving after acknowledgement. The latter two include initial scheduling. Legacy variants and all five clocks remain in JSON. Native differential testing found and fixed an empty-checkpoint normalization difference before timing. Ordinary release UI and a full merge gate remain required if this withheld candidate is reconsidered.


## Bounded conversation layout (withheld)

One private optimized WASM binary compares full legacy reads/saves with indexed ranges and revision-checked row mutations. The one-message-per-row format and its schema upgrade remain unmerged: large-history pages and incremental mutations improve, but complete history reads and some small reads regress. A page is 77 messages, not T3 timeline items. There are 76 cells with six retained observations after one warmup, counterbalanced by case, operation and implementation. These local storage clocks exclude mounted UI, TaskSaves scheduling, physical cold I/O, providers and network.

The raw runner records exit 1/pass=false because its final assertion expected 546 intervals. A separate exact-matrix audit accepts all 532 actual intervals: both the 2-message and 50-message fixtures lack earlier pages. Original raw evidence is preserved. Counts, full content/float readback and transaction event order pass. The private full-read path does not audit rows outside the declared interval, so production corruption handling remains incomplete.

| History | Storage operation | Version | Median ms | Range ms | Raw evidence |
| --- | --- | --- | ---: | ---: | --- |
| small-control | tail | baseline | 0.300 | 0.300–0.400 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| small-control | tail | indexed | 0.900 | 0.800–1.000 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| small-control | earlier | baseline | 0.300 | 0.200–0.400 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| small-control | earlier | indexed | 0.400 | 0.300–0.500 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| small-control | full | baseline | 0.300 | 0.200–0.400 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| small-control | full | indexed | 1.000 | 1.000–1.200 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| small-control | append | baseline | 0.550 | 0.500–0.700 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| small-control | append | indexed | 0.300 | 0.200–0.400 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| small-control | replace | baseline | 0.600 | 0.500–0.900 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| small-control | replace | indexed | 0.500 | 0.300–0.700 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| mixed-41k-parts | tail | baseline | 57.200 | 53.900–58.700 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| mixed-41k-parts | tail | indexed | 11.250 | 10.900–22.300 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| mixed-41k-parts | earlier | baseline | 57.300 | 55.500–58.300 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| mixed-41k-parts | earlier | indexed | 10.750 | 10.400–14.800 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| mixed-41k-parts | full | baseline | 55.050 | 54.300–55.900 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| mixed-41k-parts | full | indexed | 63.050 | 60.800–63.700 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| mixed-41k-parts | append | baseline | 111.850 | 106.300–122.100 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| mixed-41k-parts | append | indexed | 0.500 | 0.300–0.600 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| mixed-41k-parts | replace | baseline | 114.500 | 108.100–156.300 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| mixed-41k-parts | replace | indexed | 1.200 | 1.100–1.300 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| messages-45k | tail | baseline | 66.700 | 63.700–70.700 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| messages-45k | tail | indexed | 1.100 | 1.000–1.200 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| messages-45k | earlier | baseline | 62.550 | 61.100–66.700 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| messages-45k | earlier | indexed | 1.100 | 1.000–10.100 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| messages-45k | full | baseline | 63.200 | 59.800–75.200 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| messages-45k | full | indexed | 355.000 | 346.700–363.100 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| messages-45k | append | baseline | 116.000 | 108.700–149.600 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| messages-45k | append | indexed | 0.550 | 0.400–0.800 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| messages-45k | replace | baseline | 114.650 | 112.300–155.100 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| messages-45k | replace | indexed | 0.600 | 0.400–0.700 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| messages-75k | tail | baseline | 107.350 | 105.500–130.400 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| messages-75k | tail | indexed | 1.150 | 1.000–1.200 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| messages-75k | earlier | baseline | 108.750 | 105.200–127.400 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| messages-75k | earlier | indexed | 1.100 | 1.000–15.600 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| messages-75k | full | baseline | 103.550 | 99.800–109.400 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| messages-75k | full | indexed | 606.500 | 585.600–616.100 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| messages-75k | append | baseline | 197.100 | 191.300–203.900 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| messages-75k | append | indexed | 0.500 | 0.400–0.700 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| messages-75k | replace | baseline | 199.000 | 193.600–225.300 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| messages-75k | replace | indexed | 0.550 | 0.400–0.700 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| large-tool-output | tail | baseline | 15.700 | 13.800–21.200 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| large-tool-output | tail | indexed | 11.600 | 11.400–35.500 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| large-tool-output | earlier | baseline | 13.850 | 13.500–35.500 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| large-tool-output | earlier | indexed | 3.600 | 3.500–4.100 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| large-tool-output | full | baseline | 14.900 | 13.600–16.500 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| large-tool-output | full | indexed | 14.750 | 14.600–17.600 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| large-tool-output | append | baseline | 25.000 | 24.000–34.000 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| large-tool-output | append | indexed | 0.600 | 0.400–0.700 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| large-tool-output | replace | baseline | 26.200 | 24.100–38.300 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| large-tool-output | replace | indexed | 1.150 | 1.000–1.400 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| long-reasoning | tail | baseline | 2.500 | 2.400–2.600 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| long-reasoning | tail | indexed | 2.700 | 2.600–2.800 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| long-reasoning | full | baseline | 2.500 | 2.300–2.700 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| long-reasoning | full | indexed | 2.700 | 2.500–2.800 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| long-reasoning | append | baseline | 5.700 | 5.500–6.500 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| long-reasoning | append | indexed | 0.500 | 0.400–0.600 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| long-reasoning | replace | baseline | 6.000 | 5.600–6.700 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| long-reasoning | replace | indexed | 5.900 | 5.800–6.700 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| escaped-output | tail | baseline | 5.250 | 5.000–10.800 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| escaped-output | tail | indexed | 6.500 | 6.400–8.900 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| escaped-output | full | baseline | 5.350 | 5.100–6.500 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| escaped-output | full | indexed | 6.300 | 6.000–6.400 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| escaped-output | append | baseline | 12.250 | 11.800–13.500 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| escaped-output | append | indexed | 0.500 | 0.500–0.600 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| escaped-output | replace | baseline | 12.400 | 11.900–14.100 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| escaped-output | replace | indexed | 0.850 | 0.500–0.900 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| structured-fallback | tail | baseline | 0.550 | 0.500–2.100 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| structured-fallback | tail | indexed | 0.950 | 0.800–1.100 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| structured-fallback | earlier | baseline | 0.500 | 0.400–0.600 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| structured-fallback | earlier | indexed | 0.450 | 0.400–0.500 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| structured-fallback | full | baseline | 0.500 | 0.500–0.600 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| structured-fallback | full | indexed | 1.150 | 1.100–1.500 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| structured-fallback | append | baseline | 1.100 | 0.900–1.400 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| structured-fallback | append | indexed | 0.400 | 0.300–0.500 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| structured-fallback | replace | baseline | 1.100 | 1.000–1.100 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| structured-fallback | replace | indexed | 0.400 | 0.300–0.500 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |

### Initial indexed preparation (single intervals)

One warm complete sidecar preparation per history, retaining the original legacy blob. These eight observations are outside repeated read/write clocks and are not repeated distributions. The 75,000-message preparation costs about 1.95 seconds. Indexed revision checks require every official writer and reader to migrate together behind a version barrier; web storage changes also leave native desktop persistence work open.

| History | Messages | Preparation ms (n=1) | Raw evidence |
| --- | ---: | ---: | --- |
| small-control | 100 | 5.900 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| mixed-41k-parts | 460 | 170.400 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| messages-45k | 45000 | 1132.700 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| messages-75k | 75000 | 1946.300 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| large-tool-output | 100 | 38.800 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| long-reasoning | 2 | 8.200 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| escaped-output | 50 | 20.100 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |
| structured-fallback | 100 | 3.900 | [web-conversation-bounded3-2026-11-13](target/performance-bounded3-timings.json) |

## Fixed message-count chunk sweep (not adopted)

One optimized private WASM binary compares whole-record storage with 1, 16, 128 and 1,024 messages per indexed row. All 760 intervals pass; 190 cells have three retained samples after one warmup, with alternating order. This is an exploratory grid, not a selected production budget. Large chunks recover full reads but erase mutation gains on content-heavy histories; small chunks retain bounded mutation work but add full-read overhead. All unfavorable cells remain below.

Read clocks include request/decode/strict range and cardinality work, owned projection and terminal transaction completion. Complete snapshots also count all indexed thread rows, rejecting unexpected rows outside the range. Mutation clocks include encoding and parent/revision/affected-original checks through chunk/head writes and commit. Expected loading and caller draft construction precede clocks; result marshaling/hash/verification/restore, mounted UI, TaskSaves scheduling, physical cold I/O and providers/network are excluded. Per-record IDs and cross-size content/float parity pass. The earlier ID-comparison failure is preserved separately; no failed data is promoted.

| History | Operation | Version | Median ms | Range ms | Raw evidence |
| --- | --- | --- | ---: | ---: | --- |
| small-control | tail | baseline | 0.400 | 0.300–0.400 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | tail | indexed-1 | 0.800 | 0.800–1.100 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | tail | indexed-16 | 0.400 | 0.400–0.400 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | tail | indexed-128 | 0.400 | 0.400–0.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | tail | indexed-1024 | 0.400 | 0.300–0.400 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | earlier | baseline | 0.300 | 0.300–0.400 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | earlier | indexed-1 | 0.400 | 0.300–0.400 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | earlier | indexed-16 | 0.400 | 0.300–0.400 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | earlier | indexed-128 | 0.400 | 0.400–0.400 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | earlier | indexed-1024 | 0.400 | 0.300–0.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | full | baseline | 0.300 | 0.300–0.400 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | full | indexed-1 | 1.200 | 1.100–1.300 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | full | indexed-16 | 0.500 | 0.500–0.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | full | indexed-128 | 0.400 | 0.400–0.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | full | indexed-1024 | 0.500 | 0.400–0.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | append | baseline | 0.800 | 0.700–0.900 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | append | indexed-1 | 0.500 | 0.300–0.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | append | indexed-16 | 0.500 | 0.400–0.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | append | indexed-128 | 1.000 | 1.000–1.000 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | append | indexed-1024 | 1.000 | 0.900–1.000 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | replace | baseline | 0.900 | 0.600–0.900 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | replace | indexed-1 | 0.500 | 0.400–0.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | replace | indexed-16 | 0.500 | 0.400–0.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | replace | indexed-128 | 1.000 | 0.600–1.100 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | replace | indexed-1024 | 1.200 | 0.700–1.300 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | tail | baseline | 58.100 | 56.900–59.100 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | tail | indexed-1 | 11.200 | 10.800–14.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | tail | indexed-16 | 10.400 | 10.300–10.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | tail | indexed-128 | 23.200 | 22.900–23.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | tail | indexed-1024 | 54.500 | 53.800–56.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | earlier | baseline | 56.900 | 55.500–58.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | earlier | indexed-1 | 10.700 | 10.200–10.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | earlier | indexed-16 | 8.900 | 8.900–9.000 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | earlier | indexed-128 | 14.500 | 14.400–16.200 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | earlier | indexed-1024 | 57.200 | 53.500–57.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | full | baseline | 54.500 | 54.300–57.000 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | full | indexed-1 | 64.300 | 62.700–77.100 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | full | indexed-16 | 53.300 | 52.400–54.000 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | full | indexed-128 | 54.000 | 50.900–112.800 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | full | indexed-1024 | 54.600 | 53.300–55.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | append | baseline | 110.500 | 109.600–117.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | append | indexed-1 | 0.600 | 0.400–0.900 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | append | indexed-16 | 3.500 | 3.300–3.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | append | indexed-128 | 18.600 | 18.400–18.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | append | indexed-1024 | 110.800 | 106.500–112.100 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | replace | baseline | 111.400 | 111.300–112.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | replace | indexed-1 | 1.400 | 1.300–1.400 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | replace | indexed-16 | 3.800 | 3.700–3.900 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | replace | indexed-128 | 19.700 | 18.500–20.200 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | replace | indexed-1024 | 115.700 | 105.900–117.200 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | tail | baseline | 62.800 | 62.600–63.900 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | tail | indexed-1 | 1.100 | 0.900–1.400 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | tail | indexed-16 | 0.600 | 0.500–0.800 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | tail | indexed-128 | 0.600 | 0.600–0.800 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | tail | indexed-1024 | 1.800 | 1.800–1.900 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | earlier | baseline | 64.700 | 63.100–65.100 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | earlier | indexed-1 | 0.900 | 0.900–1.000 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | earlier | indexed-16 | 0.400 | 0.400–0.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | earlier | indexed-128 | 0.500 | 0.400–0.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | earlier | indexed-1024 | 1.800 | 1.500–1.900 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | full | baseline | 64.500 | 64.000–67.800 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | full | indexed-1 | 352.500 | 346.200–356.200 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | full | indexed-16 | 95.900 | 94.100–97.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | full | indexed-128 | 81.200 | 78.000–89.400 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | full | indexed-1024 | 61.400 | 58.900–63.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | append | baseline | 117.500 | 115.800–119.000 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | append | indexed-1 | 18.100 | 0.600–23.800 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | append | indexed-16 | 0.600 | 0.600–0.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | append | indexed-128 | 1.000 | 1.000–1.100 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | append | indexed-1024 | 3.200 | 3.000–3.300 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | replace | baseline | 117.800 | 116.100–118.200 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | replace | indexed-1 | 0.700 | 0.600–0.800 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | replace | indexed-16 | 0.500 | 0.400–0.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | replace | indexed-128 | 1.000 | 0.800–1.200 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | replace | indexed-1024 | 3.300 | 3.100–3.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | tail | baseline | 114.200 | 106.000–117.100 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | tail | indexed-1 | 0.900 | 0.800–1.100 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | tail | indexed-16 | 0.500 | 0.500–0.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | tail | indexed-128 | 0.500 | 0.500–0.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | tail | indexed-1024 | 0.800 | 0.600–0.800 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | earlier | baseline | 107.700 | 103.500–113.800 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | earlier | indexed-1 | 1.000 | 1.000–1.800 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | earlier | indexed-16 | 0.600 | 0.500–0.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | earlier | indexed-128 | 0.600 | 0.600–0.800 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | earlier | indexed-1024 | 0.800 | 0.500–0.900 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | full | baseline | 103.700 | 101.800–111.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | full | indexed-1 | 583.000 | 572.600–583.900 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | full | indexed-16 | 158.800 | 158.600–218.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | full | indexed-128 | 136.600 | 136.100–136.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | full | indexed-1024 | 102.900 | 98.900–103.200 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | append | baseline | 199.900 | 189.300–200.300 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | append | indexed-1 | 0.600 | 0.500–0.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | append | indexed-16 | 0.600 | 0.500–0.800 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | append | indexed-128 | 1.400 | 1.200–1.400 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | append | indexed-1024 | 1.500 | 1.400–1.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | replace | baseline | 202.000 | 185.800–202.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | replace | indexed-1 | 0.600 | 0.600–4.300 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | replace | indexed-16 | 0.500 | 0.500–0.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | replace | indexed-128 | 1.200 | 1.200–1.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | replace | indexed-1024 | 1.600 | 1.600–1.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | tail | baseline | 14.100 | 13.800–14.300 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | tail | indexed-1 | 12.600 | 12.500–12.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | tail | indexed-16 | 12.400 | 11.300–12.900 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | tail | indexed-128 | 13.500 | 13.100–13.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | tail | indexed-1024 | 16.000 | 15.500–16.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | earlier | baseline | 13.800 | 13.500–13.800 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | earlier | indexed-1 | 3.400 | 3.400–3.400 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | earlier | indexed-16 | 7.300 | 4.600–7.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | earlier | indexed-128 | 13.500 | 13.500–16.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | earlier | indexed-1024 | 13.800 | 13.400–14.000 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | full | baseline | 14.000 | 13.900–20.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | full | indexed-1 | 14.800 | 14.700–14.900 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | full | indexed-16 | 13.900 | 13.700–14.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | full | indexed-128 | 14.000 | 13.900–14.100 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | full | indexed-1024 | 13.900 | 13.300–14.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | append | baseline | 26.900 | 26.100–27.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | append | indexed-1 | 0.600 | 0.600–0.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | append | indexed-16 | 1.500 | 1.300–10.800 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | append | indexed-128 | 21.900 | 21.200–22.300 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | append | indexed-1024 | 21.300 | 20.900–23.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | replace | baseline | 25.000 | 24.800–26.900 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | replace | indexed-1 | 1.200 | 0.900–1.300 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | replace | indexed-16 | 1.600 | 1.300–2.000 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | replace | indexed-128 | 22.900 | 22.500–30.900 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | replace | indexed-1024 | 21.800 | 21.600–24.300 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| long-reasoning | tail | baseline | 2.700 | 2.700–2.900 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| long-reasoning | tail | indexed-1 | 2.900 | 2.700–3.000 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| long-reasoning | tail | indexed-16 | 2.800 | 2.600–3.100 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| long-reasoning | tail | indexed-128 | 3.000 | 2.600–3.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| long-reasoning | tail | indexed-1024 | 3.000 | 2.700–4.100 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| long-reasoning | full | baseline | 2.600 | 2.600–2.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| long-reasoning | full | indexed-1 | 2.900 | 2.900–2.900 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| long-reasoning | full | indexed-16 | 2.800 | 2.700–2.800 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| long-reasoning | full | indexed-128 | 2.700 | 2.700–2.800 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| long-reasoning | full | indexed-1024 | 2.800 | 2.700–2.900 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| long-reasoning | append | baseline | 5.800 | 5.700–5.900 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| long-reasoning | append | indexed-1 | 0.500 | 0.400–0.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| long-reasoning | append | indexed-16 | 5.500 | 5.300–5.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| long-reasoning | append | indexed-128 | 5.900 | 5.700–7.000 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| long-reasoning | append | indexed-1024 | 5.800 | 5.700–5.900 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| long-reasoning | replace | baseline | 6.000 | 5.600–6.200 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| long-reasoning | replace | indexed-1 | 6.200 | 6.100–8.000 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| long-reasoning | replace | indexed-16 | 6.000 | 5.800–6.200 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| long-reasoning | replace | indexed-128 | 6.000 | 6.000–6.200 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| long-reasoning | replace | indexed-1024 | 6.100 | 5.900–6.200 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| escaped-output | tail | baseline | 5.400 | 5.200–5.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| escaped-output | tail | indexed-1 | 6.500 | 6.500–6.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| escaped-output | tail | indexed-16 | 5.600 | 5.600–7.200 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| escaped-output | tail | indexed-128 | 5.600 | 5.500–5.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| escaped-output | tail | indexed-1024 | 5.600 | 5.500–5.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| escaped-output | full | baseline | 5.300 | 5.300–5.300 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| escaped-output | full | indexed-1 | 6.700 | 6.500–7.100 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| escaped-output | full | indexed-16 | 5.600 | 5.600–5.800 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| escaped-output | full | indexed-128 | 5.500 | 5.500–6.000 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| escaped-output | full | indexed-1024 | 6.300 | 5.900–6.800 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| escaped-output | append | baseline | 12.400 | 12.400–12.900 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| escaped-output | append | indexed-1 | 0.600 | 0.400–0.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| escaped-output | append | indexed-16 | 1.100 | 1.000–1.100 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| escaped-output | append | indexed-128 | 11.700 | 11.400–11.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| escaped-output | append | indexed-1024 | 11.700 | 11.700–11.800 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| escaped-output | replace | baseline | 12.500 | 12.500–12.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| escaped-output | replace | indexed-1 | 0.800 | 0.800–0.900 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| escaped-output | replace | indexed-16 | 1.100 | 1.000–1.300 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| escaped-output | replace | indexed-128 | 11.500 | 11.400–11.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| escaped-output | replace | indexed-1024 | 11.700 | 11.500–11.800 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | tail | baseline | 0.600 | 0.600–0.800 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | tail | indexed-1 | 1.000 | 0.900–1.100 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | tail | indexed-16 | 0.600 | 0.600–0.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | tail | indexed-128 | 0.600 | 0.600–0.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | tail | indexed-1024 | 0.600 | 0.600–0.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | earlier | baseline | 0.500 | 0.400–0.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | earlier | indexed-1 | 0.500 | 0.400–0.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | earlier | indexed-16 | 0.400 | 0.400–0.400 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | earlier | indexed-128 | 0.600 | 0.600–1.100 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | earlier | indexed-1024 | 0.600 | 0.600–0.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | full | baseline | 0.600 | 0.600–0.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | full | indexed-1 | 1.300 | 1.200–1.300 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | full | indexed-16 | 0.700 | 0.700–0.800 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | full | indexed-128 | 0.700 | 0.700–0.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | full | indexed-1024 | 0.700 | 0.600–0.800 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | append | baseline | 1.100 | 1.100–1.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | append | indexed-1 | 0.400 | 0.300–0.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | append | indexed-16 | 0.500 | 0.500–0.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | append | indexed-128 | 1.300 | 1.200–1.400 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | append | indexed-1024 | 1.300 | 1.200–1.300 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | replace | baseline | 1.200 | 1.200–1.400 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | replace | indexed-1 | 0.400 | 0.400–0.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | replace | indexed-16 | 0.400 | 0.400–0.400 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | replace | indexed-128 | 1.200 | 1.100–1.300 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | replace | indexed-1024 | 1.100 | 1.100–1.400 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |

### Chunk preparation (n=1 per case and size)

Thirty-two single warm preparations retain the legacy blob and are outside repeated operation clocks. They are not repeated migration distributions. No format/schema or application reader/writer from this experiment has merged. Native persistence, complete consumers/recovery/version barriers and full UI/network/T3 acceptance remain open.

| History | Version | Preparation ms (n=1) | Raw evidence |
| --- | --- | ---: | --- |
| small-control | indexed-1 | 5.900 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | indexed-16 | 1.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | indexed-128 | 1.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| small-control | indexed-1024 | 1.000 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | indexed-1 | 169.400 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | indexed-16 | 153.900 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | indexed-128 | 159.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| mixed-41k-parts | indexed-1024 | 202.900 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | indexed-1 | 1138.000 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | indexed-16 | 233.400 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | indexed-128 | 177.200 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-45k | indexed-1024 | 177.200 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | indexed-1 | 1951.900 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | indexed-16 | 390.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | indexed-128 | 299.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| messages-75k | indexed-1024 | 354.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | indexed-1 | 38.000 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | indexed-16 | 37.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | indexed-128 | 40.000 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| large-tool-output | indexed-1024 | 42.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| long-reasoning | indexed-1 | 9.000 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| long-reasoning | indexed-16 | 9.400 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| long-reasoning | indexed-128 | 8.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| long-reasoning | indexed-1024 | 8.800 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| escaped-output | indexed-1 | 17.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| escaped-output | indexed-16 | 20.400 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| escaped-output | indexed-128 | 18.500 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| escaped-output | indexed-1024 | 18.600 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | indexed-1 | 4.100 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | indexed-16 | 1.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | indexed-128 | 1.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |
| structured-fallback | indexed-1024 | 1.700 | [web-conversation-chunk5-2026-11-13](target/performance-chunk5-timings.json) |

## Controlled task-stream request counts

The task UI now reacts to complete refresh events, ignores keepalives/unrelated events, coalesces events within a chunk and invalidates pending replies on disconnect. This fix is applied to unpublished working-tree task-workspace code and is unmerged. The exact HTML passes 508 parser controls, 32 browser stream cases, 21 behavior checks, nine adversarial checks and two disconnect regressions. Intentional invalid-token 403s are recorded separately; the final browser gate has zero unexpected errors.

In the earlier controlled-fragmentation study, real loopback HTTP returns one synthetic specification and no tasks; a browser ReadableStream controls SSE fragmentation. The eight matched remote desktop stream cases trigger 111 baseline snapshot requests and five candidate requests. Narrow-viewport and local-listener candidate cases also pass. These are request/body-byte counts from single controlled cases. That study does not measure latency, production wire bytes, real-runtime/TCP SSE or mobile/T3 acceptance. The separate actual-runtime study below keeps its own source, data and clocks.

| Listener | Width px | Input | Version | Chunks | Snapshot requests | JSON body bytes |
| --- | ---: | --- | --- | ---: | ---: | ---: |
| paired | 1280 | keepalive comment | baseline | 1 | 1 | 69806 |
| paired | 1280 | one-byte fragmented refresh | baseline | 30 | 30 | 2094180 |
| paired | 1280 | two complete refreshes in one chunk | baseline | 1 | 1 | 69806 |
| paired | 1280 | unknown data event | baseline | 1 | 1 | 69806 |
| paired | 1280 | UTF-8 comment split into bytes | baseline | 14 | 14 | 977284 |
| paired | 1280 | CRLF refresh split into bytes | baseline | 33 | 33 | 2303598 |
| paired | 1280 | CR refresh split into bytes | baseline | 30 | 30 | 2094180 |
| paired | 1280 | long data line | baseline | 1 | 1 | 69806 |
| paired | 1280 | keepalive comment | candidate | 1 | 0 | 0 |
| paired | 1280 | one-byte fragmented refresh | candidate | 30 | 1 | 69806 |
| paired | 1280 | two complete refreshes in one chunk | candidate | 1 | 1 | 69806 |
| paired | 1280 | unknown data event | candidate | 1 | 0 | 0 |
| paired | 1280 | UTF-8 comment split into bytes | candidate | 14 | 0 | 0 |
| paired | 1280 | CRLF refresh split into bytes | candidate | 33 | 1 | 69806 |
| paired | 1280 | CR refresh split into bytes | candidate | 30 | 1 | 69806 |
| paired | 1280 | long data line | candidate | 1 | 1 | 69806 |
| paired | 390 | keepalive comment | candidate | 1 | 0 | 0 |
| paired | 390 | one-byte fragmented refresh | candidate | 30 | 1 | 69806 |
| paired | 390 | two complete refreshes in one chunk | candidate | 1 | 1 | 69806 |
| paired | 390 | unknown data event | candidate | 1 | 0 | 0 |
| paired | 390 | UTF-8 comment split into bytes | candidate | 14 | 0 | 0 |
| paired | 390 | CRLF refresh split into bytes | candidate | 33 | 1 | 69806 |
| paired | 390 | CR refresh split into bytes | candidate | 30 | 1 | 69806 |
| paired | 390 | long data line | candidate | 1 | 1 | 69806 |
| local | 1280 | keepalive comment | candidate | 1 | 0 | 0 |
| local | 1280 | one-byte fragmented refresh | candidate | 30 | 1 | 69806 |
| local | 1280 | two complete refreshes in one chunk | candidate | 1 | 1 | 69806 |
| local | 1280 | unknown data event | candidate | 1 | 0 | 0 |
| local | 1280 | UTF-8 comment split into bytes | candidate | 14 | 0 | 0 |
| local | 1280 | CRLF refresh split into bytes | candidate | 33 | 1 | 69806 |
| local | 1280 | CR refresh split into bytes | candidate | 30 | 1 | 69806 |
| local | 1280 | long data line | candidate | 1 | 1 | 69806 |

## Real task runtime and network timings

Frozen optimized native CoordinationRuntime and paired-task remote_router, real TCP SSE/authentication/JSON persistence, four completed synthetic tasks with eight padded messages each and one specification. The before/after HTML pair preserves the same newer feature markup. No provider/background workers run. Small title/revision metadata differences remain visible in per-sample body sizes. This unpublished feature and its performance fix remain unmerged.

Manual Refresh uses an in-page click through current DOM and two animation-frame callbacks; no physical pointer/paint claim. Three retained intervals follow one warmup per cell. Fetch body duration is the browser ResourceTiming boundary. The mobile profile is 390px, 1.6 Mbit/s down, 0.75 Mbit/s up and 300ms latency without CPU shaping, real device, TLS or proxy. Actual-runtime request suppression is verified; these samples do not establish a latency improvement or a matched T3 result. Exact response-body reductions belong to the separate controlled-fragmentation study.

| Profile | Version | Boundary | Samples | Median ms | Range ms | Raw evidence |
| --- | --- | --- | ---: | ---: | ---: | --- |
| loopback | before | Click through DOM and two frames | 3 | 25.100 | 23.800–28.800 | [web-task-runtime6-2026-11-13](target/performance-task-runtime6-timings.json) |
| loopback | before | Fetch through body | 3 | 1.800 | 1.600–2.600 | [web-task-runtime6-2026-11-13](target/performance-task-runtime6-timings.json) |
| loopback | after | Click through DOM and two frames | 3 | 27.900 | 27.500–28.900 | [web-task-runtime6-2026-11-13](target/performance-task-runtime6-timings.json) |
| loopback | after | Fetch through body | 3 | 1.600 | 1.600–2.700 | [web-task-runtime6-2026-11-13](target/performance-task-runtime6-timings.json) |
| mobile_1_6mbps_300ms | after | Click through DOM and two frames | 3 | 779.200 | 778.500–779.300 | [web-task-runtime6-2026-11-13](target/performance-task-runtime6-timings.json) |
| mobile_1_6mbps_300ms | after | Fetch through body | 3 | 753.100 | 750.200–755.100 | [web-task-runtime6-2026-11-13](target/performance-task-runtime6-timings.json) |
| mobile_1_6mbps_300ms | before | Click through DOM and two frames | 3 | 779.000 | 778.900–779.500 | [web-task-runtime6-2026-11-13](target/performance-task-runtime6-timings.json) |
| mobile_1_6mbps_300ms | before | Fetch through body | 3 | 752.100 | 751.000–755.800 | [web-task-runtime6-2026-11-13](target/performance-task-runtime6-timings.json) |

A real Axum keepalive comment produces one snapshot GET before the fix and zero after it (one observed case per version). There is no production wire-byte total. Thirty-five controls pass, including owner/guest scoping, origin/token rejection, expiry, real SaveSpec persistence, draft/focus preservation and revoked-stream reader closure. Four separate notification diagnostics retain their single-interval arming/control-round-trip boundary in JSON and the linked report. The native child and all test browser contexts exit cleanly.

| Version | Actual keepalive read chunks | Snapshot GETs | Raw evidence |
| --- | ---: | ---: | --- |
| before | 1 | 1 | [Native runtime](target/performance-task-runtime6-browser.json) |
| after | 1 | 0 | [Native runtime](target/performance-task-runtime6-browser.json) |

## Fresh device pairing

The unpublished task HTML now reuses its first authorized snapshot instead of downloading it again before showing tasks. The initial stream refresh remains to catch changes between snapshot and subscription. Pairing ownership is separate from snapshot-read ordering, so manual Refresh can finish during pairing and delayed old pairing replies cannot undo Disconnect or a newer attempt.

The measured before/after HTML pair uses the same unchanged 88,264-byte snapshot from the frozen actual runtime used in the earlier study, with identical private forwarding adapters. The changing root backend is outside that build. Three retained samples follow one warmup per cell, in fresh browser sessions. Navigation and token entry precede the in-page form-submission clock. These clocks do not establish physical pointer/paint, whole-action or T3 equivalence. Loopback frame medians are higher in this small sample and ranges overlap; the impaired-link gain and traffic reduction do not imply a universal latency win.

| Profile | Version | Endpoint | Median ms | Range ms | Raw evidence |
| --- | --- | --- | ---: | ---: | --- |
| loopback | candidate | First usable DOM and two frames | 36.600 | 27.700–38.700 | [web-task-pair1-2026-11-13](target/performance-task-pair1-timings.json) |
| loopback | candidate | Initial refresh settled and two frames | 55.300 | 54.500–61.300 | [web-task-pair1-2026-11-13](target/performance-task-pair1-timings.json) |
| loopback | before | First usable DOM and two frames | 31.800 | 30.400–44.700 | [web-task-pair1-2026-11-13](target/performance-task-pair1-timings.json) |
| loopback | before | Initial refresh settled and two frames | 48.500 | 44.700–63.900 | [web-task-pair1-2026-11-13](target/performance-task-pair1-timings.json) |
| mobile_1_6mbps_300ms | before | First usable DOM and two frames | 1531.800 | 1530.500–1538.500 | [web-task-pair1-2026-11-13](target/performance-task-pair1-timings.json) |
| mobile_1_6mbps_300ms | before | Initial refresh settled and two frames | 2613.200 | 2604.700–2615.200 | [web-task-pair1-2026-11-13](target/performance-task-pair1-timings.json) |
| mobile_1_6mbps_300ms | candidate | First usable DOM and two frames | 781.900 | 780.300–782.400 | [web-task-pair1-2026-11-13](target/performance-task-pair1-timings.json) |
| mobile_1_6mbps_300ms | candidate | Initial refresh settled and two frames | 1848.500 | 1846.900–1849.400 | [web-task-pair1-2026-11-13](target/performance-task-pair1-timings.json) |

Every retained fresh-pairing trial uses three snapshot GETs before versus two after, with 264,792 versus 176,528 JSON body bytes. These exclude HTML navigation, HTTP/TCP overhead and the open SSE stream. Saved-token startup still uses two GETs. Independent browser verification passes 23 candidate checks, reproduces four expected baseline failures, and rejects unmatched errors. The existing local/guest project browser flow also passes. Source hashes, exact body parity and terminal process evidence are linked in the [pairing report](docs/performance-task-pairing-2026-11-13.md).


## Snapshot transport compression

The unmerged working-tree paired-task router is configured to negotiate gzip Fastest for successful finite responses whose uncompressed bodies are at least 768 bytes. The minimum comes from actual guest-proposal response controls: non-saving response TCP payload cases occurred through 512 bytes; 768 was the first tested size saving traffic in all four styles and three repetitions. These counts include HTTP headers and chunk framing, excluding IP/TCP headers and TLS. Seventy-two final-policy paired controls confirm unchanged traffic below the floor and positive savings in the tested larger cases. SSE/images/errors remain uncompressed; the optional favicon request receives an empty 204 response.

The matrix below uses the earlier default 32-byte predicate, identity/Fastest/Default in one pinned optimized binary, five immutable payloads and the same task HTML. It has 480 retained native HTTP intervals plus 90 retained UI intervals, with 105 warmups. Native time includes complete encoded loopback HTTP, not isolated server CPU. UI ends after DOM update and two frames. Mobile is 390px, 1.6 Mbit/s down, 0.75 Mbit/s up and 300ms CDP latency, without CPU shaping. The 397-byte row is below the final policy floor and is prototype-only. These are not exact final-policy/full-current-root, physical-device/TLS/proxy, whole-action or matched T3 timings.

| Payload | Transport | Encoded / raw body B | Native HTTP median ms (n=32) | Local UI median ms (n=3) | Mobile UI median ms (n=3) |
| --- | --- | ---: | ---: | ---: | ---: |
| zero00 (below final floor) | identity | 397 / 397 | 0.114 | 30.5 | 331.0 |
| zero00 (below final floor) | fastest | 310 / 397 | 0.130 | 31.0 | 331.1 |
| zero00 (below final floor) | default | 304 / 397 | 0.133 | 30.6 | 340.1 |
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

Fastest was selected to limit native per-request work while retaining the main impaired-link gain. Default saved more bytes and was about 27ms faster for the largest repetitive UI fixture; it cost more native HTTP work. Small UI differences and overlapping local frame ranges do not prove a universal latency win. A 131-check prototype browser gate, 49 checks on the exact final router and 72 final byte controls pass. Original favicon failures and intermediate negative small-proposal results remain preserved. The router is tested over the older archived backend; the changing root, providers/concurrency and the full integration gate remain open. See the [compression report](docs/performance-task-compression-2026-11-13.md).


## Discovered entrypoints

These identities overlap across surfaces and are discovery starting points. A view or binding is not automatically one complete user action. “Extraction pending” means older partial measurements exist but their numeric distributions have not yet been normalized into this log. Functional checks and request counts remain distinct from measured latency.

| Kind | Entrypoint | Source | Partial records | Timing status |
| --- | --- | --- | ---: | --- |
| http | DELETE /api/connections/attempts/{id} | [main.rs:507](bruhs-server/src/main.rs:507) | 0 | measurement_pending |
| http | DELETE /api/connections/{provider} | [main.rs:503](bruhs-server/src/main.rs:503) | 0 | measurement_pending |
| http | DELETE /api/coordination/access/{id} | [main.rs:366](bruhs-server/src/main.rs:366) | 0 | measurement_pending |
| http | DELETE /api/credentials/{id} | [main.rs:511](bruhs-server/src/main.rs:511) | 0 | measurement_pending |
| http | DELETE /api/databases/{id} | [main.rs:442](bruhs-server/src/main.rs:442) | 0 | measurement_pending |
| http | DELETE /api/delivery/watches/{id} | [delivery.rs:33](bruhs-server/src/routes/delivery.rs:33) | 0 | measurement_pending |
| http | DELETE /api/local-tools/grants/{id} | [main.rs:373](bruhs-server/src/main.rs:373) | 0 | measurement_pending |
| http | DELETE /api/meetings/{id} | [main.rs:635](bruhs-server/src/main.rs:635) | 1 | historical_timing_extraction_pending |
| http | DELETE /api/meetings/{id}/generations/{request_id} | [main.rs:671](bruhs-server/src/main.rs:671) | 2 | historical_timing_extraction_pending |
| http | DELETE /api/meetings/{id}/segments/{segment_id} | [main.rs:655](bruhs-server/src/main.rs:655) | 1 | historical_timing_extraction_pending |
| http | DELETE /api/tasks/{id} | [main.rs:626](bruhs-server/src/main.rs:626) | 2 | historical_timing_extraction_pending |
| http | DELETE /api/terminal/sessions/{id} | [main.rs:524](bruhs-server/src/main.rs:524) | 0 | measurement_pending |
| http | GET /api/agent-extensions/config | [main.rs:336](bruhs-server/src/main.rs:336) | 0 | measurement_pending |
| http | GET /api/agent-extensions/ui | [main.rs:332](bruhs-server/src/main.rs:332) | 0 | measurement_pending |
| http | GET /api/agent-extensions/usage | [main.rs:340](bruhs-server/src/main.rs:340) | 0 | measurement_pending |
| http | GET /api/agent/artifacts/{id} | [main.rs:551](bruhs-server/src/main.rs:551) | 0 | measurement_pending |
| http | GET /api/agent/artifacts/{id}/diagnostics | [main.rs:559](bruhs-server/src/main.rs:559) | 0 | measurement_pending |
| http | GET /api/agent/artifacts/{id}/preview | [main.rs:555](bruhs-server/src/main.rs:555) | 0 | measurement_pending |
| http | GET /api/agent/auth | [main.rs:480](bruhs-server/src/main.rs:480) | 0 | measurement_pending |
| http | GET /api/agent/auth/{provider} | [main.rs:481](bruhs-server/src/main.rs:481) | 0 | measurement_pending |
| http | GET /api/agent/media | [main.rs:563](bruhs-server/src/main.rs:563) | 0 | measurement_pending |
| http | GET /api/agent/rich-media | [main.rs:593](bruhs-server/src/main.rs:593) | 0 | measurement_pending |
| http | GET /api/agent/rich-response | [main.rs:589](bruhs-server/src/main.rs:589) | 0 | measurement_pending |
| http | GET /api/agent/rich-response/asset/{name} | [main.rs:576](bruhs-server/src/main.rs:576) | 0 | measurement_pending |
| http | GET /api/agent/rich-response/state | [main.rs:568](bruhs-server/src/main.rs:568) | 0 | measurement_pending |
| http | GET /api/agent/uploads/open | [main.rs:550](bruhs-server/src/main.rs:550) | 0 | measurement_pending |
| http | GET /api/api-client/agent/proposals/{id} | [main.rs:430](bruhs-server/src/main.rs:430) | 0 | measurement_pending |
| http | GET /api/api-client/collection | [main.rs:408](bruhs-server/src/main.rs:408) | 0 | measurement_pending |
| http | GET /api/api-client/proposals | [main.rs:421](bruhs-server/src/main.rs:421) | 0 | measurement_pending |
| http | GET /api/calendar/events/stream | [main.rs:746](bruhs-server/src/main.rs:746) | 0 | measurement_pending |
| http | GET /api/calendar/google/calendars | [main.rs:738](bruhs-server/src/main.rs:738) | 0 | measurement_pending |
| http | GET /api/calendar/google/oauth/callback | [main.rs:722](bruhs-server/src/main.rs:722) | 0 | measurement_pending |
| http | GET /api/calendar/google/state | [main.rs:730](bruhs-server/src/main.rs:730) | 0 | measurement_pending |
| http | GET /api/capture/activity-monitor | [main.rs:758](bruhs-server/src/main.rs:758) | 2 | historical_timing_extraction_pending |
| http | GET /api/capture/status | [main.rs:750](bruhs-server/src/main.rs:750) | 2 | historical_timing_extraction_pending |
| http | GET /api/connections | [main.rs:498](bruhs-server/src/main.rs:498) | 0 | measurement_pending |
| http | GET /api/connections/attempts/{id} | [main.rs:507](bruhs-server/src/main.rs:507) | 0 | measurement_pending |
| http | GET /api/connections/{provider} | [main.rs:503](bruhs-server/src/main.rs:503) | 0 | measurement_pending |
| http | GET /api/coordination | [main.rs:348](bruhs-server/src/main.rs:348) | 1 | request_counts_recorded_latency_pending |
| http | GET /api/coordination/access | [main.rs:362](bruhs-server/src/main.rs:362) | 0 | measurement_pending |
| http | GET /api/coordination/changes | [main.rs:358](bruhs-server/src/main.rs:358) | 0 | measurement_pending |
| http | GET /api/coordination/ui | [main.rs:349](bruhs-server/src/main.rs:349) | 0 | measurement_pending |
| http | GET /api/credentials | [main.rs:497](bruhs-server/src/main.rs:497) | 0 | measurement_pending |
| http | GET /api/databases | [main.rs:403](bruhs-server/src/main.rs:403) | 0 | measurement_pending |
| http | GET /api/databases/agent | [main.rs:444](bruhs-server/src/main.rs:444) | 0 | measurement_pending |
| http | GET /api/databases/agent/{id}/tables | [main.rs:449](bruhs-server/src/main.rs:449) | 0 | measurement_pending |
| http | GET /api/databases/{id}/tables | [main.rs:443](bruhs-server/src/main.rs:443) | 0 | measurement_pending |
| http | GET /api/delivery | [delivery.rs:17](bruhs-server/src/routes/delivery.rs:17) | 0 | measurement_pending |
| http | GET /api/delivery/ui | [delivery.rs:16](bruhs-server/src/routes/delivery.rs:16) | 0 | measurement_pending |
| http | GET /api/events/stream | [main.rs:621](bruhs-server/src/main.rs:621) | 2 | historical_timing_extraction_pending |
| http | GET /api/health | [main.rs:485](bruhs-server/src/main.rs:485) | 2 | historical_timing_extraction_pending |
| http | GET /api/import/sessions | [main.rs:532](bruhs-server/src/main.rs:532) | 0 | measurement_pending |
| http | GET /api/import/thread | [main.rs:533](bruhs-server/src/main.rs:533) | 0 | measurement_pending |
| http | GET /api/local-tools | [main.rs:370](bruhs-server/src/main.rs:370) | 0 | measurement_pending |
| http | GET /api/local-tools/devices | [main.rs:385](bruhs-server/src/main.rs:385) | 0 | measurement_pending |
| http | GET /api/local-tools/devices/video | [main.rs:393](bruhs-server/src/main.rs:393) | 0 | measurement_pending |
| http | GET /api/local-tools/ui | [main.rs:371](bruhs-server/src/main.rs:371) | 0 | measurement_pending |
| http | GET /api/meetings | [main.rs:627](bruhs-server/src/main.rs:627) | 3 | historical_timing_extraction_pending |
| http | GET /api/meetings/stream | [main.rs:684](bruhs-server/src/main.rs:684) | 4 | bounded_variant_observations |
| http | GET /api/meetings/{id} | [main.rs:633](bruhs-server/src/main.rs:633) | 3 | historical_timing_extraction_pending |
| http | GET /api/meetings/{id}/generations | [main.rs:676](bruhs-server/src/main.rs:676) | 2 | historical_timing_extraction_pending |
| http | GET /api/meetings/{id}/generations/{request_id} | [main.rs:671](bruhs-server/src/main.rs:671) | 2 | historical_timing_extraction_pending |
| http | GET /api/meetings/{id}/local-focus | [main.rs:628](bruhs-server/src/main.rs:628) | 3 | historical_timing_extraction_pending |
| http | GET /api/meetings/{id}/segment-removals | [main.rs:659](bruhs-server/src/main.rs:659) | 1 | historical_timing_extraction_pending |
| http | GET /api/meetings/{id}/segments | [main.rs:651](bruhs-server/src/main.rs:651) | 0 | measurement_pending |
| http | GET /api/people | [main.rs:688](bruhs-server/src/main.rs:688) | 3 | historical_timing_extraction_pending |
| http | GET /api/people/versions | [main.rs:700](bruhs-server/src/main.rs:700) | 3 | historical_timing_extraction_pending |
| http | GET /api/people/{id} | [main.rs:692](bruhs-server/src/main.rs:692) | 3 | historical_timing_extraction_pending |
| http | GET /api/preview/servers | [main.rs:528](bruhs-server/src/main.rs:528) | 0 | measurement_pending |
| http | GET /api/reactive | [main.rs:401](bruhs-server/src/main.rs:401) | 0 | measurement_pending |
| http | GET /api/reactive/sources/{id}/token | [main.rs:459](bruhs-server/src/main.rs:459) | 0 | measurement_pending |
| http | GET /api/remote-hosts | [remote_hosts.rs:15](bruhs-server/src/routes/remote_hosts.rs:15) | 0 | measurement_pending |
| http | GET /api/remote-hosts/ui | [remote_hosts.rs:17](bruhs-server/src/routes/remote_hosts.rs:17) | 0 | measurement_pending |
| http | GET /api/tasks | [main.rs:620](bruhs-server/src/main.rs:620) | 2 | historical_timing_extraction_pending |
| http | GET /api/tasks/stream | [main.rs:625](bruhs-server/src/main.rs:625) | 2 | historical_timing_extraction_pending |
| http | GET /api/terminal/sessions | [main.rs:520](bruhs-server/src/main.rs:520) | 0 | measurement_pending |
| http | GET /api/terminal/ws | [main.rs:519](bruhs-server/src/main.rs:519) | 1 | bounded_variant_observations |
| http | GET /api/updates | [main.rs:486](bruhs-server/src/main.rs:486) | 0 | measurement_pending |
| http | GET /api/weekly-reports | [main.rs:453](bruhs-server/src/main.rs:453) | 0 | measurement_pending |
| http | GET /api/weekly-reports/{id} | [main.rs:458](bruhs-server/src/main.rs:458) | 0 | measurement_pending |
| http | GET /api/workbench/ui | [main.rs:328](bruhs-server/src/main.rs:328) | 0 | measurement_pending |
| http | PATCH /api/calendar/google/calendars/{id} | [main.rs:742](bruhs-server/src/main.rs:742) | 0 | measurement_pending |
| http | PATCH /api/meetings/{id} | [main.rs:634](bruhs-server/src/main.rs:634) | 1 | historical_timing_extraction_pending |
| http | PATCH /api/meetings/{id}/skip_observations | [main.rs:680](bruhs-server/src/main.rs:680) | 1 | historical_timing_extraction_pending |
| http | PATCH /api/meetings/{id}/speakers/{speaker_id} | [main.rs:643](bruhs-server/src/main.rs:643) | 1 | historical_timing_extraction_pending |
| http | PATCH /api/people/{id} | [main.rs:692](bruhs-server/src/main.rs:692) | 1 | historical_timing_extraction_pending |
| http | PATCH /api/people/{id}/rename | [main.rs:706](bruhs-server/src/main.rs:706) | 1 | historical_timing_extraction_pending |
| http | PATCH /api/people/{id}/tracked | [main.rs:702](bruhs-server/src/main.rs:702) | 1 | historical_timing_extraction_pending |
| http | POST /api/agent | [main.rs:469](bruhs-server/src/main.rs:469) | 0 | measurement_pending |
| http | POST /api/agent-extensions/config | [main.rs:336](bruhs-server/src/main.rs:336) | 0 | measurement_pending |
| http | POST /api/agent-extensions/history/preview | [main.rs:320](bruhs-server/src/main.rs:320) | 0 | measurement_pending |
| http | POST /api/agent-extensions/history/{id}/commit | [main.rs:324](bruhs-server/src/main.rs:324) | 0 | measurement_pending |
| http | POST /api/agent-extensions/usage/refresh | [main.rs:344](bruhs-server/src/main.rs:344) | 0 | measurement_pending |
| http | POST /api/agent/approve | [main.rs:491](bruhs-server/src/main.rs:491) | 0 | measurement_pending |
| http | POST /api/agent/artifacts/{id}/diagnostics | [main.rs:559](bruhs-server/src/main.rs:559) | 0 | measurement_pending |
| http | POST /api/agent/branch-pull | [main.rs:541](bruhs-server/src/main.rs:541) | 0 | measurement_pending |
| http | POST /api/agent/files | [main.rs:535](bruhs-server/src/main.rs:535) | 0 | measurement_pending |
| http | POST /api/agent/files/children | [main.rs:536](bruhs-server/src/main.rs:536) | 3 | historical_timing_extraction_pending |
| http | POST /api/agent/git-branches | [main.rs:597](bruhs-server/src/main.rs:597) | 3 | historical_timing_extraction_pending |
| http | POST /api/agent/git-checkout | [main.rs:598](bruhs-server/src/main.rs:598) | 0 | measurement_pending |
| http | POST /api/agent/git-diff | [main.rs:609](bruhs-server/src/main.rs:609) | 3 | historical_timing_extraction_pending |
| http | POST /api/agent/git-status | [main.rs:608](bruhs-server/src/main.rs:608) | 3 | historical_timing_extraction_pending |
| http | POST /api/agent/list-dir | [main.rs:610](bruhs-server/src/main.rs:610) | 3 | historical_timing_extraction_pending |
| http | POST /api/agent/login | [main.rs:615](bruhs-server/src/main.rs:615) | 0 | measurement_pending |
| http | POST /api/agent/login/{provider} | [main.rs:616](bruhs-server/src/main.rs:616) | 0 | measurement_pending |
| http | POST /api/agent/prompt-suggestion | [main.rs:470](bruhs-server/src/main.rs:470) | 0 | measurement_pending |
| http | POST /api/agent/pulls | [main.rs:540](bruhs-server/src/main.rs:540) | 0 | measurement_pending |
| http | POST /api/agent/rich-response/download | [main.rs:585](bruhs-server/src/main.rs:585) | 0 | measurement_pending |
| http | POST /api/agent/rich-response/export | [main.rs:580](bruhs-server/src/main.rs:580) | 0 | measurement_pending |
| http | POST /api/agent/rich-response/media | [main.rs:572](bruhs-server/src/main.rs:572) | 0 | measurement_pending |
| http | POST /api/agent/rich-response/remote-media | [main.rs:564](bruhs-server/src/main.rs:564) | 0 | measurement_pending |
| http | POST /api/agent/second-opinion | [main.rs:476](bruhs-server/src/main.rs:476) | 0 | measurement_pending |
| http | POST /api/agent/secret | [main.rs:492](bruhs-server/src/main.rs:492) | 0 | measurement_pending |
| http | POST /api/agent/skills | [main.rs:545](bruhs-server/src/main.rs:545) | 0 | measurement_pending |
| http | POST /api/agent/steer | [main.rs:475](bruhs-server/src/main.rs:475) | 0 | measurement_pending |
| http | POST /api/agent/thread-reply | [main.rs:493](bruhs-server/src/main.rs:493) | 0 | measurement_pending |
| http | POST /api/agent/uploads | [main.rs:546](bruhs-server/src/main.rs:546) | 0 | measurement_pending |
| http | POST /api/agent/validate-path | [main.rs:611](bruhs-server/src/main.rs:611) | 3 | historical_timing_extraction_pending |
| http | POST /api/agent/worktree | [main.rs:599](bruhs-server/src/main.rs:599) | 0 | measurement_pending |
| http | POST /api/agent/worktree-restore | [main.rs:604](bruhs-server/src/main.rs:604) | 0 | measurement_pending |
| http | POST /api/agent/worktree-snapshot | [main.rs:600](bruhs-server/src/main.rs:600) | 0 | measurement_pending |
| http | POST /api/api-client/agent/propose | [main.rs:426](bruhs-server/src/main.rs:426) | 0 | measurement_pending |
| http | POST /api/api-client/cancel/{id} | [main.rs:417](bruhs-server/src/main.rs:417) | 0 | measurement_pending |
| http | POST /api/api-client/collection | [main.rs:408](bruhs-server/src/main.rs:408) | 0 | measurement_pending |
| http | POST /api/api-client/credential | [main.rs:413](bruhs-server/src/main.rs:413) | 0 | measurement_pending |
| http | POST /api/api-client/decision | [main.rs:425](bruhs-server/src/main.rs:425) | 0 | measurement_pending |
| http | POST /api/api-client/send | [main.rs:412](bruhs-server/src/main.rs:412) | 0 | measurement_pending |
| http | POST /api/calendar/google/disconnect | [main.rs:726](bruhs-server/src/main.rs:726) | 0 | measurement_pending |
| http | POST /api/calendar/google/oauth/start | [main.rs:718](bruhs-server/src/main.rs:718) | 0 | measurement_pending |
| http | POST /api/calendar/google/sync | [main.rs:734](bruhs-server/src/main.rs:734) | 0 | measurement_pending |
| http | POST /api/capture/activity-monitor | [main.rs:758](bruhs-server/src/main.rs:758) | 2 | historical_timing_extraction_pending |
| http | POST /api/capture/start | [main.rs:756](bruhs-server/src/main.rs:756) | 2 | historical_timing_extraction_pending |
| http | POST /api/capture/stop | [main.rs:757](bruhs-server/src/main.rs:757) | 2 | historical_timing_extraction_pending |
| http | POST /api/connections/{provider}/start | [main.rs:499](bruhs-server/src/main.rs:499) | 0 | measurement_pending |
| http | POST /api/coordination/access | [main.rs:362](bruhs-server/src/main.rs:362) | 0 | measurement_pending |
| http | POST /api/coordination/commands | [main.rs:350](bruhs-server/src/main.rs:350) | 0 | measurement_pending |
| http | POST /api/coordination/wait | [main.rs:354](bruhs-server/src/main.rs:354) | 0 | measurement_pending |
| http | POST /api/databases | [main.rs:403](bruhs-server/src/main.rs:403) | 0 | measurement_pending |
| http | POST /api/databases/agent/describe | [main.rs:437](bruhs-server/src/main.rs:437) | 0 | measurement_pending |
| http | POST /api/databases/agent/query | [main.rs:445](bruhs-server/src/main.rs:445) | 0 | measurement_pending |
| http | POST /api/databases/browse | [main.rs:435](bruhs-server/src/main.rs:435) | 0 | measurement_pending |
| http | POST /api/databases/cancel/{id} | [main.rs:441](bruhs-server/src/main.rs:441) | 0 | measurement_pending |
| http | POST /api/databases/describe | [main.rs:436](bruhs-server/src/main.rs:436) | 0 | measurement_pending |
| http | POST /api/databases/grant | [main.rs:407](bruhs-server/src/main.rs:407) | 0 | measurement_pending |
| http | POST /api/databases/query | [main.rs:434](bruhs-server/src/main.rs:434) | 0 | measurement_pending |
| http | POST /api/delivery/cancel | [delivery.rs:31](bruhs-server/src/routes/delivery.rs:31) | 0 | measurement_pending |
| http | POST /api/delivery/events/{id}/acknowledge | [delivery.rs:34](bruhs-server/src/routes/delivery.rs:34) | 0 | measurement_pending |
| http | POST /api/delivery/execute | [delivery.rs:22](bruhs-server/src/routes/delivery.rs:22) | 0 | measurement_pending |
| http | POST /api/delivery/execute-native-stack | [delivery.rs:23](bruhs-server/src/routes/delivery.rs:23) | 0 | measurement_pending |
| http | POST /api/delivery/native-stack-status | [delivery.rs:27](bruhs-server/src/routes/delivery.rs:27) | 0 | measurement_pending |
| http | POST /api/delivery/prepare | [delivery.rs:21](bruhs-server/src/routes/delivery.rs:21) | 0 | measurement_pending |
| http | POST /api/delivery/read | [delivery.rs:18](bruhs-server/src/routes/delivery.rs:18) | 0 | measurement_pending |
| http | POST /api/delivery/trackers | [delivery.rs:20](bruhs-server/src/routes/delivery.rs:20) | 0 | measurement_pending |
| http | POST /api/delivery/trackers/read | [delivery.rs:19](bruhs-server/src/routes/delivery.rs:19) | 0 | measurement_pending |
| http | POST /api/delivery/watches | [delivery.rs:32](bruhs-server/src/routes/delivery.rs:32) | 0 | measurement_pending |
| http | POST /api/export/t3 | [main.rs:534](bruhs-server/src/main.rs:534) | 0 | measurement_pending |
| http | POST /api/local-tools/browser/command | [main.rs:381](bruhs-server/src/main.rs:381) | 0 | measurement_pending |
| http | POST /api/local-tools/browser/start | [main.rs:377](bruhs-server/src/main.rs:377) | 0 | measurement_pending |
| http | POST /api/local-tools/devices/command | [main.rs:389](bruhs-server/src/main.rs:389) | 0 | measurement_pending |
| http | POST /api/local-tools/dictation | [main.rs:397](bruhs-server/src/main.rs:397) | 0 | measurement_pending |
| http | POST /api/local-tools/grants | [main.rs:372](bruhs-server/src/main.rs:372) | 0 | measurement_pending |
| http | POST /api/meetings | [main.rs:632](bruhs-server/src/main.rs:632) | 1 | historical_timing_extraction_pending |
| http | POST /api/meetings/{id}/chat | [main.rs:663](bruhs-server/src/main.rs:663) | 0 | measurement_pending |
| http | POST /api/meetings/{id}/local-focus | [main.rs:628](bruhs-server/src/main.rs:628) | 3 | historical_timing_extraction_pending |
| http | POST /api/meetings/{id}/speakers/{speaker_id}/link/{person_id} | [main.rs:647](bruhs-server/src/main.rs:647) | 1 | historical_timing_extraction_pending |
| http | POST /api/meetings/{id}/stop | [main.rs:639](bruhs-server/src/main.rs:639) | 1 | historical_timing_extraction_pending |
| http | POST /api/meetings/{id}/summarize | [main.rs:667](bruhs-server/src/main.rs:667) | 2 | historical_timing_extraction_pending |
| http | POST /api/open-in-editor | [main.rs:515](bruhs-server/src/main.rs:515) | 0 | measurement_pending |
| http | POST /api/people | [main.rs:688](bruhs-server/src/main.rs:688) | 1 | historical_timing_extraction_pending |
| http | POST /api/people/profiles | [main.rs:701](bruhs-server/src/main.rs:701) | 3 | historical_timing_extraction_pending |
| http | POST /api/people/resolve-merges | [main.rs:696](bruhs-server/src/main.rs:696) | 3 | historical_timing_extraction_pending |
| http | POST /api/people/{id}/enroll | [main.rs:714](bruhs-server/src/main.rs:714) | 1 | historical_timing_extraction_pending |
| http | POST /api/people/{source_id}/merge_into/{target_id} | [main.rs:710](bruhs-server/src/main.rs:710) | 1 | historical_timing_extraction_pending |
| http | POST /api/reactive/commands | [main.rs:402](bruhs-server/src/main.rs:402) | 0 | measurement_pending |
| http | POST /api/reactive/hooks/{id} | [main.rs:463](bruhs-server/src/main.rs:463) | 0 | measurement_pending |
| http | POST /api/remote-hosts/commands | [remote_hosts.rs:16](bruhs-server/src/routes/remote_hosts.rs:16) | 0 | measurement_pending |
| http | POST /api/terminal/sessions | [main.rs:520](bruhs-server/src/main.rs:520) | 0 | measurement_pending |
| http | POST /api/updates/providers/{provider} | [main.rs:487](bruhs-server/src/main.rs:487) | 0 | measurement_pending |
| http | POST /api/weekly-reports/commands | [main.rs:454](bruhs-server/src/main.rs:454) | 0 | measurement_pending |
| http | PUT /api/agent/rich-response/state | [main.rs:568](bruhs-server/src/main.rs:568) | 0 | measurement_pending |
| http | PUT /api/credentials/{id} | [main.rs:511](bruhs-server/src/main.rs:511) | 0 | measurement_pending |
| http | paired_task:GET / | [coordination.rs:162](bruhs-server/src/routes/coordination.rs:162) | 0 | measurement_pending |
| http | paired_task:GET /api/changes | [coordination.rs:173](bruhs-server/src/routes/coordination.rs:173) | 1 | request_counts_recorded_latency_pending |
| http | paired_task:GET /api/tasks | [coordination.rs:170](bruhs-server/src/routes/coordination.rs:170) | 4 | bounded_variant_observations |
| http | paired_task:GET /favicon.ico | [coordination.rs:163](bruhs-server/src/routes/coordination.rs:163) | 1 | functional_checks_recorded_latency_pending |
| http | paired_task:GET /icon-512.png | [coordination.rs:167](bruhs-server/src/routes/coordination.rs:167) | 0 | measurement_pending |
| http | paired_task:GET /manifest.webmanifest | [coordination.rs:164](bruhs-server/src/routes/coordination.rs:164) | 0 | measurement_pending |
| http | paired_task:POST /api/commands | [coordination.rs:171](bruhs-server/src/routes/coordination.rs:171) | 1 | functional_checks_recorded_latency_pending |
| http | paired_task:POST /api/proposals | [coordination.rs:172](bruhs-server/src/routes/coordination.rs:172) | 1 | functional_checks_recorded_latency_pending |
| mcp | bruhs_agent_audit | [tools.rs:188](bruhs-mcp/src/tools.rs:188) | 3 | historical_timing_extraction_pending |
| mcp | bruhs_list_tasks | [tools.rs:177](bruhs-mcp/src/tools.rs:177) | 1 | historical_timing_extraction_pending |
| mcp | bruhs_list_threads | [tools.rs:172](bruhs-mcp/src/tools.rs:172) | 1 | historical_timing_extraction_pending |
| mcp | bruhs_propose_workflow | [workflow_schema.rs:49](bruhs-mcp/src/tools/workflow_schema.rs:49) | 1 | historical_timing_extraction_pending |
| mcp | bruhs_read_calendar | [automation.rs:98](bruhs-models/src/models/automation.rs:98) | 6 | historical_timing_extraction_pending |
| mcp | bruhs_read_capture | [automation.rs:105](bruhs-models/src/models/automation.rs:105) | 6 | historical_timing_extraction_pending |
| mcp | bruhs_read_chat | [automation.rs:96](bruhs-models/src/models/automation.rs:96) | 7 | historical_timing_extraction_pending |
| mcp | bruhs_read_chat_messages | [tools.rs:149](bruhs-mcp/src/tools.rs:149) | 1 | historical_timing_extraction_pending |
| mcp | bruhs_read_finance | [automation.rs:103](bruhs-models/src/models/automation.rs:103) | 8 | historical_timing_extraction_pending |
| mcp | bruhs_read_health | [automation.rs:104](bruhs-models/src/models/automation.rs:104) | 6 | historical_timing_extraction_pending |
| mcp | bruhs_read_inbox | [automation.rs:101](bruhs-models/src/models/automation.rs:101) | 6 | historical_timing_extraction_pending |
| mcp | bruhs_read_knowledge | [automation.rs:100](bruhs-models/src/models/automation.rs:100) | 8 | historical_timing_extraction_pending |
| mcp | bruhs_read_meeting | [tools.rs:131](bruhs-mcp/src/tools.rs:131) | 1 | historical_timing_extraction_pending |
| mcp | bruhs_read_meeting_note | [tools.rs:124](bruhs-mcp/src/tools.rs:124) | 1 | historical_timing_extraction_pending |
| mcp | bruhs_read_meeting_notes | [tools.rs:116](bruhs-mcp/src/tools.rs:116) | 6 | historical_timing_extraction_pending |
| mcp | bruhs_read_meetings | [automation.rs:99](bruhs-models/src/models/automation.rs:99) | 6 | historical_timing_extraction_pending |
| mcp | bruhs_read_people | [automation.rs:102](bruhs-models/src/models/automation.rs:102) | 6 | historical_timing_extraction_pending |
| mcp | bruhs_read_person | [tools.rs:140](bruhs-mcp/src/tools.rs:140) | 1 | historical_timing_extraction_pending |
| mcp | bruhs_read_tasks | [automation.rs:97](bruhs-models/src/models/automation.rs:97) | 6 | historical_timing_extraction_pending |
| mcp | bruhs_request_workflow_rollback | [tools.rs:164](bruhs-mcp/src/tools.rs:164) | 1 | historical_timing_extraction_pending |
| mcp | bruhs_workflow_status | [tools.rs:157](bruhs-mcp/src/tools.rs:157) | 1 | historical_timing_extraction_pending |
| mcp | bruhs_write_calendar | [automation.rs:98](bruhs-models/src/models/automation.rs:98) | 1 | historical_timing_extraction_pending |
| mcp | bruhs_write_capture | [automation.rs:105](bruhs-models/src/models/automation.rs:105) | 1 | historical_timing_extraction_pending |
| mcp | bruhs_write_chat | [automation.rs:96](bruhs-models/src/models/automation.rs:96) | 1 | historical_timing_extraction_pending |
| mcp | bruhs_write_finance | [automation.rs:103](bruhs-models/src/models/automation.rs:103) | 1 | historical_timing_extraction_pending |
| mcp | bruhs_write_health | [automation.rs:104](bruhs-models/src/models/automation.rs:104) | 1 | historical_timing_extraction_pending |
| mcp | bruhs_write_inbox | [automation.rs:101](bruhs-models/src/models/automation.rs:101) | 1 | historical_timing_extraction_pending |
| mcp | bruhs_write_knowledge | [automation.rs:100](bruhs-models/src/models/automation.rs:100) | 1 | historical_timing_extraction_pending |
| mcp | bruhs_write_meetings | [automation.rs:99](bruhs-models/src/models/automation.rs:99) | 1 | historical_timing_extraction_pending |
| mcp | bruhs_write_people | [automation.rs:102](bruhs-models/src/models/automation.rs:102) | 1 | historical_timing_extraction_pending |
| mcp | bruhs_write_tasks | [automation.rs:97](bruhs-models/src/models/automation.rs:97) | 1 | historical_timing_extraction_pending |
| mcp_method | initialize | [serve.rs:160](bruhs-mcp/src/serve.rs:160) | 1 | historical_timing_extraction_pending |
| mcp_method | ping | [serve.rs:178](bruhs-mcp/src/serve.rs:178) | 0 | measurement_pending |
| mcp_method | tools/call | [serve.rs:177](bruhs-mcp/src/serve.rs:177) | 2 | historical_timing_extraction_pending |
| mcp_method | tools/list | [serve.rs:171](bruhs-mcp/src/serve.rs:171) | 1 | historical_timing_extraction_pending |
| palette | desktop:capture | [palette_ui.rs:192](bruhs-desktop/src/views/workspace/palette_ui.rs:192) | 0 | measurement_pending |
| palette | desktop:continue-in-t3 | [palette_ui.rs:132](bruhs-desktop/src/views/workspace/palette_ui.rs:132) | 0 | measurement_pending |
| palette | desktop:continue-in-workbench | [palette_ui.rs:138](bruhs-desktop/src/views/workspace/palette_ui.rs:138) | 0 | measurement_pending |
| palette | desktop:copy-pull-link | [palette_ui.rs:144](bruhs-desktop/src/views/workspace/palette_ui.rs:144) | 0 | measurement_pending |
| palette | desktop:copy-pull-number | [palette_ui.rs:150](bruhs-desktop/src/views/workspace/palette_ui.rs:150) | 0 | measurement_pending |
| palette | desktop:copy-resume-command | [palette_ui.rs:120](bruhs-desktop/src/views/workspace/palette_ui.rs:120) | 0 | measurement_pending |
| palette | desktop:copy-thread-link | [palette_ui.rs:114](bruhs-desktop/src/views/workspace/palette_ui.rs:114) | 0 | measurement_pending |
| palette | desktop:copy-thread-markdown | [palette_ui.rs:126](bruhs-desktop/src/views/workspace/palette_ui.rs:126) | 0 | measurement_pending |
| palette | desktop:import-conversation | [palette_ui.rs:180](bruhs-desktop/src/views/workspace/palette_ui.rs:180) | 0 | measurement_pending |
| palette | desktop:import-thread | [palette_ui.rs:96](bruhs-desktop/src/views/workspace/palette_ui.rs:96) | 0 | measurement_pending |
| palette | desktop:local-servers | [palette_ui.rs:240](bruhs-desktop/src/views/workspace/palette_ui.rs:240) | 0 | measurement_pending |
| palette | desktop:model-picker | [palette_ui.rs:264](bruhs-desktop/src/views/workspace/palette_ui.rs:264) | 0 | measurement_pending |
| palette | desktop:nav-automations | [palette_ui.rs:48](bruhs-desktop/src/views/workspace/palette_ui.rs:48) | 0 | measurement_pending |
| palette | desktop:nav-calendar | [palette_ui.rs:36](bruhs-desktop/src/views/workspace/palette_ui.rs:36) | 0 | measurement_pending |
| palette | desktop:nav-chat | [palette_ui.rs:12](bruhs-desktop/src/views/workspace/palette_ui.rs:12) | 0 | measurement_pending |
| palette | desktop:nav-inbox | [palette_ui.rs:18](bruhs-desktop/src/views/workspace/palette_ui.rs:18) | 0 | measurement_pending |
| palette | desktop:nav-knowledge | [palette_ui.rs:24](bruhs-desktop/src/views/workspace/palette_ui.rs:24) | 0 | measurement_pending |
| palette | desktop:nav-life | [palette_ui.rs:42](bruhs-desktop/src/views/workspace/palette_ui.rs:42) | 0 | measurement_pending |
| palette | desktop:nav-meetings | [palette_ui.rs:54](bruhs-desktop/src/views/workspace/palette_ui.rs:54) | 0 | measurement_pending |
| palette | desktop:nav-people | [palette_ui.rs:66](bruhs-desktop/src/views/workspace/palette_ui.rs:66) | 0 | measurement_pending |
| palette | desktop:nav-search | [palette_ui.rs:72](bruhs-desktop/src/views/workspace/palette_ui.rs:72) | 0 | measurement_pending |
| palette | desktop:nav-tasks | [palette_ui.rs:30](bruhs-desktop/src/views/workspace/palette_ui.rs:30) | 0 | measurement_pending |
| palette | desktop:nav-terminal | [palette_ui.rs:78](bruhs-desktop/src/views/workspace/palette_ui.rs:78) | 0 | measurement_pending |
| palette | desktop:nav-today | [palette_ui.rs:6](bruhs-desktop/src/views/workspace/palette_ui.rs:6) | 0 | measurement_pending |
| palette | desktop:nav-workbench | [palette_ui.rs:60](bruhs-desktop/src/views/workspace/palette_ui.rs:60) | 0 | measurement_pending |
| palette | desktop:new-document | [palette_ui.rs:174](bruhs-desktop/src/views/workspace/palette_ui.rs:174) | 0 | measurement_pending |
| palette | desktop:new-meeting-note | [palette_ui.rs:198](bruhs-desktop/src/views/workspace/palette_ui.rs:198) | 0 | measurement_pending |
| palette | desktop:new-task | [palette_ui.rs:156](bruhs-desktop/src/views/workspace/palette_ui.rs:156) | 0 | measurement_pending |
| palette | desktop:new-thread | [palette_ui.rs:90](bruhs-desktop/src/views/workspace/palette_ui.rs:90) | 1 | historical_timing_extraction_pending |
| palette | desktop:new-web-clip | [palette_ui.rs:186](bruhs-desktop/src/views/workspace/palette_ui.rs:186) | 0 | measurement_pending |
| palette | desktop:open-project | [palette_ui.rs:84](bruhs-desktop/src/views/workspace/palette_ui.rs:84) | 0 | measurement_pending |
| palette | desktop:open-terminal | [palette_ui.rs:234](bruhs-desktop/src/views/workspace/palette_ui.rs:234) | 0 | measurement_pending |
| palette | desktop:pin-thread | [palette_ui.rs:102](bruhs-desktop/src/views/workspace/palette_ui.rs:102) | 0 | measurement_pending |
| palette | desktop:plan-day | [palette_ui.rs:162](bruhs-desktop/src/views/workspace/palette_ui.rs:162) | 0 | measurement_pending |
| palette | desktop:settings | [palette_ui.rs:270](bruhs-desktop/src/views/workspace/palette_ui.rs:270) | 0 | measurement_pending |
| palette | desktop:settle-thread | [palette_ui.rs:108](bruhs-desktop/src/views/workspace/palette_ui.rs:108) | 0 | measurement_pending |
| palette | desktop:smart-scheduling | [palette_ui.rs:168](bruhs-desktop/src/views/workspace/palette_ui.rs:168) | 0 | measurement_pending |
| palette | desktop:stash-prompt | [palette_ui.rs:258](bruhs-desktop/src/views/workspace/palette_ui.rs:258) | 0 | measurement_pending |
| palette | desktop:toggle-diff | [palette_ui.rs:210](bruhs-desktop/src/views/workspace/palette_ui.rs:210) | 0 | measurement_pending |
| palette | desktop:toggle-files | [palette_ui.rs:252](bruhs-desktop/src/views/workspace/palette_ui.rs:252) | 0 | measurement_pending |
| palette | desktop:toggle-panel | [palette_ui.rs:216](bruhs-desktop/src/views/workspace/palette_ui.rs:216) | 0 | measurement_pending |
| palette | desktop:toggle-preview | [palette_ui.rs:246](bruhs-desktop/src/views/workspace/palette_ui.rs:246) | 0 | measurement_pending |
| palette | desktop:toggle-sidebar | [palette_ui.rs:204](bruhs-desktop/src/views/workspace/palette_ui.rs:204) | 0 | measurement_pending |
| palette | desktop:toggle-tasks | [palette_ui.rs:222](bruhs-desktop/src/views/workspace/palette_ui.rs:222) | 0 | measurement_pending |
| palette | desktop:toggle-terminal | [palette_ui.rs:228](bruhs-desktop/src/views/workspace/palette_ui.rs:228) | 0 | measurement_pending |
| palette | web:Capture | [command_bar.rs:266](bruhs-web/src/components/command_bar.rs:266) | 0 | measurement_pending |
| palette | web:ContinueInT3 | [command_bar.rs:206](bruhs-web/src/components/command_bar.rs:206) | 0 | measurement_pending |
| palette | web:ContinueInWorkbench | [command_bar.rs:212](bruhs-web/src/components/command_bar.rs:212) | 0 | measurement_pending |
| palette | web:CopyPullLink | [command_bar.rs:218](bruhs-web/src/components/command_bar.rs:218) | 0 | measurement_pending |
| palette | web:CopyPullNumber | [command_bar.rs:224](bruhs-web/src/components/command_bar.rs:224) | 0 | measurement_pending |
| palette | web:CopyResumeCommand | [command_bar.rs:194](bruhs-web/src/components/command_bar.rs:194) | 1 | bounded_variant_observations |
| palette | web:CopyThreadLink | [command_bar.rs:188](bruhs-web/src/components/command_bar.rs:188) | 0 | measurement_pending |
| palette | web:CopyThreadMarkdown | [command_bar.rs:200](bruhs-web/src/components/command_bar.rs:200) | 0 | measurement_pending |
| palette | web:ImportConversation | [command_bar.rs:254](bruhs-web/src/components/command_bar.rs:254) | 0 | measurement_pending |
| palette | web:ImportThread | [command_bar.rs:170](bruhs-web/src/components/command_bar.rs:170) | 0 | measurement_pending |
| palette | web:ModelSelect | [command_bar.rs:340](bruhs-web/src/components/command_bar.rs:340) | 0 | measurement_pending |
| palette | web:NavAutomations | [command_bar.rs:122](bruhs-web/src/components/command_bar.rs:122) | 0 | measurement_pending |
| palette | web:NavCalendar | [command_bar.rs:110](bruhs-web/src/components/command_bar.rs:110) | 0 | measurement_pending |
| palette | web:NavChat | [command_bar.rs:86](bruhs-web/src/components/command_bar.rs:86) | 0 | measurement_pending |
| palette | web:NavInbox | [command_bar.rs:92](bruhs-web/src/components/command_bar.rs:92) | 0 | measurement_pending |
| palette | web:NavKnowledge | [command_bar.rs:98](bruhs-web/src/components/command_bar.rs:98) | 0 | measurement_pending |
| palette | web:NavLife | [command_bar.rs:116](bruhs-web/src/components/command_bar.rs:116) | 0 | measurement_pending |
| palette | web:NavMeetings | [command_bar.rs:128](bruhs-web/src/components/command_bar.rs:128) | 0 | measurement_pending |
| palette | web:NavPeople | [command_bar.rs:140](bruhs-web/src/components/command_bar.rs:140) | 0 | measurement_pending |
| palette | web:NavSearch | [command_bar.rs:146](bruhs-web/src/components/command_bar.rs:146) | 0 | measurement_pending |
| palette | web:NavTasks | [command_bar.rs:104](bruhs-web/src/components/command_bar.rs:104) | 0 | measurement_pending |
| palette | web:NavTerminal | [command_bar.rs:152](bruhs-web/src/components/command_bar.rs:152) | 0 | measurement_pending |
| palette | web:NavToday | [command_bar.rs:80](bruhs-web/src/components/command_bar.rs:80) | 0 | measurement_pending |
| palette | web:NavWorkbench | [command_bar.rs:134](bruhs-web/src/components/command_bar.rs:134) | 0 | measurement_pending |
| palette | web:NewDocument | [command_bar.rs:248](bruhs-web/src/components/command_bar.rs:248) | 0 | measurement_pending |
| palette | web:NewMeetingNote | [command_bar.rs:272](bruhs-web/src/components/command_bar.rs:272) | 0 | measurement_pending |
| palette | web:NewTask | [command_bar.rs:230](bruhs-web/src/components/command_bar.rs:230) | 0 | measurement_pending |
| palette | web:NewTerminal | [command_bar.rs:310](bruhs-web/src/components/command_bar.rs:310) | 0 | measurement_pending |
| palette | web:NewThread | [command_bar.rs:164](bruhs-web/src/components/command_bar.rs:164) | 0 | measurement_pending |
| palette | web:NewWebClip | [command_bar.rs:260](bruhs-web/src/components/command_bar.rs:260) | 0 | measurement_pending |
| palette | web:OpenInEditor | [command_bar.rs:334](bruhs-web/src/components/command_bar.rs:334) | 0 | measurement_pending |
| palette | web:OpenProject | [command_bar.rs:158](bruhs-web/src/components/command_bar.rs:158) | 0 | measurement_pending |
| palette | web:PinThread | [command_bar.rs:176](bruhs-web/src/components/command_bar.rs:176) | 1 | bounded_variant_observations |
| palette | web:PlanDay | [command_bar.rs:236](bruhs-web/src/components/command_bar.rs:236) | 0 | measurement_pending |
| palette | web:Settings | [command_bar.rs:346](bruhs-web/src/components/command_bar.rs:346) | 0 | measurement_pending |
| palette | web:SettleThread | [command_bar.rs:182](bruhs-web/src/components/command_bar.rs:182) | 0 | measurement_pending |
| palette | web:SmartScheduling | [command_bar.rs:242](bruhs-web/src/components/command_bar.rs:242) | 0 | measurement_pending |
| palette | web:StashPrompt | [command_bar.rs:328](bruhs-web/src/components/command_bar.rs:328) | 0 | measurement_pending |
| palette | web:ToggleDiff | [command_bar.rs:286](bruhs-web/src/components/command_bar.rs:286) | 0 | measurement_pending |
| palette | web:ToggleFiles | [command_bar.rs:322](bruhs-web/src/components/command_bar.rs:322) | 0 | measurement_pending |
| palette | web:TogglePanel | [command_bar.rs:292](bruhs-web/src/components/command_bar.rs:292) | 0 | measurement_pending |
| palette | web:TogglePreview | [command_bar.rs:316](bruhs-web/src/components/command_bar.rs:316) | 0 | measurement_pending |
| palette | web:ToggleSidebar | [command_bar.rs:280](bruhs-web/src/components/command_bar.rs:280) | 0 | measurement_pending |
| palette | web:ToggleTasks | [command_bar.rs:298](bruhs-web/src/components/command_bar.rs:298) | 0 | measurement_pending |
| palette | web:ToggleTerminal | [command_bar.rs:304](bruhs-web/src/components/command_bar.rs:304) | 0 | measurement_pending |
| slash | /clear | [commands.rs:35](bruhs-models/src/logic/commands.rs:35) | 0 | measurement_pending |
| slash | /compact | [commands.rs:55](bruhs-models/src/logic/commands.rs:55) | 0 | measurement_pending |
| slash | /goal | [commands.rs:51](bruhs-models/src/logic/commands.rs:51) | 0 | measurement_pending |
| slash | /handoff | [commands.rs:59](bruhs-models/src/logic/commands.rs:59) | 0 | measurement_pending |
| slash | /help | [commands.rs:39](bruhs-models/src/logic/commands.rs:39) | 0 | measurement_pending |
| slash | /model | [commands.rs:43](bruhs-models/src/logic/commands.rs:43) | 0 | measurement_pending |
| slash | /role | [commands.rs:67](bruhs-models/src/logic/commands.rs:67) | 0 | measurement_pending |
| slash | /second-opinion | [commands.rs:63](bruhs-models/src/logic/commands.rs:63) | 0 | measurement_pending |
| slash | /settings | [commands.rs:47](bruhs-models/src/logic/commands.rs:47) | 0 | measurement_pending |
| view | Automations | [ui.rs:23](bruhs-models/src/models/ui.rs:23) | 0 | measurement_pending |
| view | Booking | [ui.rs:43](bruhs-models/src/models/ui.rs:43) | 0 | measurement_pending |
| view | Calendar | [ui.rs:21](bruhs-models/src/models/ui.rs:21) | 0 | measurement_pending |
| view | Chat | [ui.rs:19](bruhs-models/src/models/ui.rs:19) | 25 | bounded_variant_observations |
| view | ClipDetail | [ui.rs:46](bruhs-models/src/models/ui.rs:46) | 0 | measurement_pending |
| view | ConversationDetail | [ui.rs:40](bruhs-models/src/models/ui.rs:40) | 0 | measurement_pending |
| view | DocumentDetail | [ui.rs:37](bruhs-models/src/models/ui.rs:37) | 0 | measurement_pending |
| view | Inbox | [ui.rs:17](bruhs-models/src/models/ui.rs:17) | 0 | measurement_pending |
| view | Knowledge | [ui.rs:18](bruhs-models/src/models/ui.rs:18) | 0 | measurement_pending |
| view | Life | [ui.rs:22](bruhs-models/src/models/ui.rs:22) | 0 | measurement_pending |
| view | MeetingDetail | [ui.rs:30](bruhs-models/src/models/ui.rs:30) | 2 | historical_timing_extraction_pending |
| view | MeetingNoteDetail | [ui.rs:34](bruhs-models/src/models/ui.rs:34) | 0 | measurement_pending |
| view | Meetings | [ui.rs:26](bruhs-models/src/models/ui.rs:26) | 0 | measurement_pending |
| view | People | [ui.rs:27](bruhs-models/src/models/ui.rs:27) | 0 | measurement_pending |
| view | Search | [ui.rs:50](bruhs-models/src/models/ui.rs:50) | 0 | measurement_pending |
| view | Settings | [ui.rs:28](bruhs-models/src/models/ui.rs:28) | 0 | measurement_pending |
| view | Tasks | [ui.rs:20](bruhs-models/src/models/ui.rs:20) | 0 | measurement_pending |
| view | Terminal | [ui.rs:52](bruhs-models/src/models/ui.rs:52) | 0 | measurement_pending |
| view | Today | [ui.rs:16](bruhs-models/src/models/ui.rs:16) | 0 | measurement_pending |
| view | Whiteboards | [ui.rs:25](bruhs-models/src/models/ui.rs:25) | 0 | measurement_pending |
| view | Workbench | [ui.rs:24](bruhs-models/src/models/ui.rs:24) | 0 | measurement_pending |
| workflow | AddCapture | [automation.rs:256](bruhs-models/src/models/automation.rs:256) | 0 | measurement_pending |
| workflow | AddFinanceTransaction | [automation.rs:246](bruhs-models/src/models/automation.rs:246) | 0 | measurement_pending |
| workflow | CreateCalendarEvent | [automation.rs:221](bruhs-models/src/models/automation.rs:221) | 0 | measurement_pending |
| workflow | CreateChatThread | [automation.rs:215](bruhs-models/src/models/automation.rs:215) | 0 | measurement_pending |
| workflow | CreateKnowledgeNote | [automation.rs:231](bruhs-models/src/models/automation.rs:231) | 0 | measurement_pending |
| workflow | CreateMeetingNote | [automation.rs:226](bruhs-models/src/models/automation.rs:226) | 0 | measurement_pending |
| workflow | CreatePersonReminder | [automation.rs:239](bruhs-models/src/models/automation.rs:239) | 0 | measurement_pending |
| workflow | CreateTask | [automation.rs:218](bruhs-models/src/models/automation.rs:218) | 0 | measurement_pending |
| workflow | DraftInboxMessage | [automation.rs:235](bruhs-models/src/models/automation.rs:235) | 0 | measurement_pending |
| workflow | LogWorkout | [automation.rs:252](bruhs-models/src/models/automation.rs:252) | 0 | measurement_pending |

## Completion requirements

1. Review newly discovered network, palette and view entrypoints semantically and measure their required variants; source discovery does not establish supported-action or network completeness.
2. Review semantic user actions beyond source-discovered identities, including keyboard/pointer/context-menu, lifecycle/error/cancellation/retry variants and all supported clients.
3. Normalize historical timing receipts into typed observations with exact raw pointers.
4. Measure unmatched actions and T3 workloads with equivalent data, functionality, clocks and network conditions.
5. Verify final build/source context and every required action before declaring completion.
