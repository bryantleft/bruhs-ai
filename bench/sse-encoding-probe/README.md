# Task SSE constructor study

This standalone probe compares task-event construction methods and checks their exact raw SSE output. It helped identify repeated per-field framing work, the memory cost of a full temporary JSON string and the Unicode cost of checked string adapters. It measures the constructor, not a complete HTTP request.

From the repository root, run:

```sh
python3 bench/sse-encoding-probe/run.py target/task-sse-study-new
```

Use a new output directory under `target`, relative to the repository root. The runner retains source snapshots, build/test logs, executable hashes, raw intervals and separate allocation observations. It builds both executables before measuring, checks for other builders/performance executables and holds Cargo locks during four fresh processes: two timing processes and two allocation processes. Forward and reverse runs change method and case order. It never launches the application or calls providers.

The probe imports the actual `bruhs-core` task wire-model definitions. Ordinary and escaped fixtures contain 0, 1, 2, 8, 32, 200 or 10,001 tasks. Additional single-task fixtures contain a long CRLF log, a 64 KiB ASCII field or a 64 KiB Unicode field. All run as named and unnamed events, giving 34 case/count variants. Generated IDs and times are fixed synthetic data; frame hashes need no normalization.

Nine methods cover the original `json_data`, a full JSON-string temporary, five checked buffer sizes, a SIMD-checked buffer and the borrowed byte-writer buffer. The 8 KiB byte writer is the selected application candidate. The earlier checked-buffer sweep covered 256 through 16,384 bytes; the byte adapter is validated at 8 KiB. That configuration is not a universal optimum or an event-size limit.

Timing retains 30 constructor intervals after five warmups per method/case/process. Across the two timing processes there are 18,360 primary intervals and 3,060 excluded warmups. Turning the returned `Event` into an HTTP/SSE body, adding its final frame separator, collecting that body, raw-frame validation, hashing and event destruction are outside those intervals. The allocation executable instead records constructor and destruction together in each observation, with 612 observations across the two orders. It does not produce timing samples.

The counting allocator measures allocation/reallocation counts, total requested bytes and peak live requested bytes. A realloc adds its full new requested size to the total, not just its growth. Peak live bytes use the requested allocation sizes; allocator rounding and internal realloc copies are unmeasured. Input models already exist before counters start. Results exclude model cloning, networking, client framing, browser work and RSS. A fixed staging buffer does not bound the output buffer: changing chunk sizes can alter its growth and increase or decrease peak requested bytes.

Every measured event matches independently framed model JSON byte for byte. Additional contracts compare the original API for pretty raw JSON, all small buffer boundaries, split UTF-8, invalid/incomplete checked input and partial serialization failures. Counts must reproduce in both orders, with every measured allocation freed. A counter failure or protocol mismatch stops the run.

`comparison.json` keeps all raw samples, nearest-rank p50/p95/p99 and allocation counts. With 30 samples, p99 is the maximum observation. The study establishes no latency tolerance, equivalence or whole-action acceptance. The actual router's finite/streaming measurements remain a separate required check; see [the performance audit](../../docs/performance-goal-2026-11-09.md#buffered-task-event-encoding).
