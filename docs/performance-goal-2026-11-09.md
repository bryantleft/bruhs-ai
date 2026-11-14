# Action performance goal — 2026-11-09

The goal is to audit every action, remove verified bottlenecks, and validate the
result. **The goal is active.** This records the implementation passes and
the remaining work; it is not an end-to-end latency acceptance or a claim that
every action is already optimal.

The network layer is part of every action audit: connection setup/reuse, DNS/TLS,
remote and provider/proxy hops, transferred bytes, streams/backpressure,
timeouts/retries/cancellation, concurrent traffic and impaired links. Current
loopback evidence is partial; remote-network acceptance remains open.

## Evidence

Baseline for the initial table: checkout
`879e32fc93b14bb12de87c532bb75febca324674`, including the user's existing
uncommitted edits. Those measurements used optimized Rust binaries on arm64
macOS 26.3 and deterministic synthetic records. The initial table measures local
computation, excluding rendering, storage, CLI startup, and network time.
Later passes distinguish computation, native test layout, and browser layout.
Other workspace builds were running, so the timing fixtures are diagnostic,
not hardware-independent CI thresholds.

| Action fixture | Before p50 / p95 (ms) | After p50 / p95 (ms) |
| --- | ---: | ---: |
| Thread ordering, 10,000 threads | 102.148 / 105.063 | 2.306 / 2.841 |
| Find absent text, 500 × ~2 KB messages | 56.508 / 59.587 | 2.084 / 3.813 |
| Find repeated text, same messages | 62.048 / 63.679 | 2.879 / 3.296 |
| Diff appended line, 1,500 lines | 6.089 / 6.537 | 0.083 / 0.084 |
| Diff unchanged, 1,500 lines | 6.113 / 6.584 | 0.083 / 0.084 |
| Universal search, 10,000 matches / 50 results | 0.496 / 0.590 | 0.278 / 0.282 |
| Day planning, 10,000 tasks | — / 2.444 | — / 0.128 |

The existing domain benchmarks also ran. Their current p95 values are
2.515 ms for inbox follow-ups, 0.825 ms for knowledge backlinks, 1.078 ms
for life insights, 0.148 ms for scheduling, and 0.022 ms for automation
preview/capability checks. No speedup is attributed to this pass for those
unchanged algorithms.

Reproduce:

```sh
cargo bench -p bruhs-models --bench actions --locked
cargo bench -p bruhs-models --locked
cargo test -p bruhs-web --release --locked sidebar_navigation_cost -- --ignored --nocapture
scripts/gate.sh --quick
```

Raw session output is under `target/performance-*.log`; gate step output is
under `target/gate/`. These are generated artifacts and can be cleaned.

## Large-record pass

The second pass measured the inbox, transcript preparation, and native snapshot
construction. All records remain stored and accessible. Both inbox surfaces now
mount at most 100 compact conversation cards per page; opening a card still
loads its complete conversation. Web search indexes every participant field and
full message body once when records change, instead of lowercasing and cloning
the matching histories on every keystroke. Unchanged keyed cards retain their
DOM nodes. Preview strings are capped at 160 Unicode scalars to cover their
single truncated display line; the search index retains the complete text.

| Local computation fixture | Before p50 / p95 (ms) | After p50 / p95 (ms) |
| --- | ---: | ---: |
| Inbox query, 10,000 histories of ~3 KB | 5.607 / 6.537 | 0.820 / 1.008 |
| Transcript preparation, 50,000 segments | 17.873 / 20.284 | 7.261 / 7.480 |

Cold construction of the inbox directory costs **7.801 / 9.019 ms** p50/p95
in the same fixture. This is an up-front normalization/memory tradeoff, not a
claim that cold indexing itself is faster. The query comparison includes the
old full-history clone/sort and the new indexed query/100-card projection; it
excludes DOM work. Transcript preparation now clones each borrowed segment once
and moves owned segments directly into speaker groups, eliminating intermediate
copies. A regression verifies grouping order, speaker boundaries, every segment
field, and preservation of owned text allocations. Full web transcript DOM
rendering remains a separate, unbounded path to address.

The native GPUI test-frame fixture found a severe rendering slope in the old
inbox. The same optimized fixture now stays bounded as stored records grow:

| Conversations | Before p50 / p95 (ms) | After p50 / p95 (ms) |
| --- | ---: | ---: |
| 100 | 9.594 / 10.709 | 11.140 / 11.900 |
| 1,000 | 101.830 / 106.687 | 11.326 / 12.305 |
| 10,000 | 1,281.572 / 1,347.672 | 11.308 / 12.578 |

These are offscreen test-frame costs, not GPU presentation or live input-to-paint
measurements. The small 100-record fixture did not improve; the result is the
removal of the growth in rendering cost, not a speedup claim for every data size.
The 100-row page budget remains below a 16.7 ms frame in this fixture. Pointer
regressions reach the last page, open its conversation, retain its complete
message, and clamp the page after records disappear.

`Workspace::snapshot` with 5,000 saved chat messages (~19 MB of text) costs
**1.072 / 1.324 ms** p50/p95 in the first run and **1.178 / 1.636 ms** in the
follow-up run on the UI thread. This fixture includes creation and destruction
of the owned snapshot; production hands that snapshot to a worker, so it does
not isolate creation alone. It excludes serialization and file writes; no
snapshot optimization was made or speedup claimed. It does not justify
changing the acknowledged save/recovery
protocol; meeting/knowledge-heavy snapshots remain to measure.

The asynchronous terminal follow-up now invalidates pending focus/error results
when a user selects or closes another tab, and rechecks selection before deferred
focus. A GPUI regression covers both races without starting a shell.

Reproduce the additional diagnostics:

```sh
cargo bench -p bruhs-models --bench inbox_browse --locked
cargo test -p bruhs-web --release --locked transcript_grouping_cost -- --ignored --nocapture
cargo test -p bruhs-desktop --bin bruhs-desktop --release --locked large_action_cost -- --ignored --nocapture
```

## Native library and chat pass

The third pass measured task list/board, knowledge library, and chat redraws in
optimized GPUI test windows. The inbox result did not cover these independent
renderers. Their old costs also grew with every stored record:

| Fixture | Baseline p50 / p95 (ms) |
| --- | ---: |
| Task list, 100 / 1,000 / 10,000 tasks | 5.777 / 8.468; 60.012 / 65.910; 850.430 / 876.899 |
| Task board, same counts in one column | 9.772 / 10.044; 95.281 / 101.177; 1,114.598 / 1,183.387 |
| Knowledge, 100 / 1,000 / 10,000 documents | 5.537 / 5.732; 55.216 / 57.753; 721.850 / 737.428 |
| Chat, 100 / 1,000 / 5,000 plain user messages | 3.260 / 3.640; 25.055 / 26.337; 129.155 / 134.169 |

The first follow-up immediately after the release build had intermittent
outliers (task p95 values up to 141.265 ms and dense-board p95 102.326 ms),
alongside median task costs of 3.556–6.015 ms. Concurrent builds were running;
the cause of those outliers was not isolated. Three repetitions of the unchanged
optimized binary then produced the following **range of per-run p95 values**:

| Fixture | Follow-up p95 ranges at the same record counts (ms) |
| --- | ---: |
| Task list, 100 / 1,000 / 10,000 | 4.750–5.131 / 4.815–5.489 / 5.059–5.339 |
| Task board, same counts in one column | 3.924–4.422 / 3.961–4.417 / 3.829–4.522 |
| Knowledge, 100 / 1,000 / 10,000 documents | 1.502–1.786 / 1.485–1.756 / 1.359–1.701 |
| Chat, 100 / 1,000 / 5,000 messages | 1.259–1.294 / 1.034–1.501 / 1.151–1.434 |

A separate dense fixture uses 10,000 tasks distributed across all four columns,
with wrapped titles and due dates. It mounts at most 160 cards and measured
p95 **7.588–8.519 ms** across those three repetitions. These results establish
bounded steady-state work; they do not establish a tail-latency guarantee under
machine contention. The first run and all repetitions remain in the raw logs.

Task pages now borrow records and retain their complete descriptions; board
columns share a single filtering/grouping pass. Pointer and Enter navigation
reach the final page. Selection follows mode/project/status changes, while an
ordinary title edit preserves the page the user chose. Document and source
pages are independent and preserve full text in their editors. Scrollbars are
added after the actual scroll children so programmatic reveal indexes address
records/sections rather than the scrollbar overlay.

Native chat now uses a variable-height list instead of constructing every
message on each redraw. Find still searches the entire history; Home/End,
page keys, and arrows navigate it. The renderer remeasures only the previously
rendered range and changed tail geometry. Regressions verify offscreen find,
full-history keyboard reachability, tail resumption, preservation of a reader's
position during streaming, and the scroll anchor when a long row shrinks.
The chat fixture uses plain user messages; it does not establish cold
Markdown/math/diagram preparation or long streaming-reply latency.

A separate optimized probe of the linked renderer libraries found the next
concrete issue: first-call math preparation took **289.428 ms**, and five new
expressions after initialization took **13.648–17.304 ms** each. Mermaid chains
of 10 / 100 / 500 edges took **231.533 / 53.686 / 1,095.999 ms** respectively
(the 10-edge case initializes the renderer). These are individual local CPU
observations, not p95 values. The probe used the library's default diagram theme
and thin LTO; it did not time the app adapter or GPU. Source inspection shows
that native cache misses at that source snapshot invoked these renderers synchronously from
message preparation. This motivated the fourth pass below; rendered history
virtualization alone did not address it. The probe source and output
are `target/performance-rich-content.rs` and `target/performance-rich-content.log`.

All timings in this pass include the test executor's redraw/layout work but
exclude GPU presentation, real OS input, network and save completion. Other
workspace builds ran concurrently. These are diagnostic measurements rather
than portable performance thresholds.

Reproduce:

```sh
cargo test -p bruhs-desktop --bin bruhs-desktop --release --locked native_library_action_cost -- --ignored --nocapture
```

## Cold native rich-content pass

The fourth pass separates source parsing from SVG generation. New math and
Mermaid sources now queue background work; the UI immediately presents their
complete selectable source alongside the surrounding prose. SVG byte hashing
and image construction also happen on the worker. Completion only adopts an
image/error and notifies the view; the existing viewport remeasures its visible
range. GPUI already rasterizes images in the background and identifies them by
their byte hash, so this is not a claim that an existing raster cache was missing.

An optimized diagnostic compares the former synchronous native adapter with
the new foreground parsing/queueing/element construction, then separately
measures a real pending-state model update and offscreen test layout:

| Source | Former foreground SVG preparation (ms) | Foreground queue preparation (ms) | Pending update + test layout (ms) |
| --- | ---: | ---: | ---: |
| First math expression | 271.599 | 0.086 | 1.348 |
| Mermaid chain, 10 edges | 12.938 | 0.037 | 0.531 |
| Mermaid chain, 100 edges | 52.939 | 0.125 | 1.157 |
| Mermaid chain, 500 edges | 1,079.019 | 0.398 | 7.949 |

These are single observations, not p95 values. They use the application's
native adapter/theme and release LTO. The old column excludes complete UI layout,
whereas the last column includes pending-source layout; they are deliberately
separate measurements. Rendering the final SVG still takes background CPU time.
The fixture pauses those workers, asserts that the pending source was actually
laid out, and measures no GPU presentation, real OS input, or network activity.
It does not establish a universal latency or frame-rate guarantee. The final
output is `target/performance-rich-release-final.log`.

Exact source, display mode, and renderer kind identify shared pending/ready
entries. Added prose tokens retain completed diagrams and stable prose block
identities. Source replacement, rewind, and thread reset cancel queued work
and prevent a running obsolete result from being adopted. Entries live while
prepared messages reference them and are released when the last reference
disappears. This avoids a fixed-size live cache repeatedly evicting/requeueing
a single message containing more than 256 formulas; the existing global renderer
LRUs keep their original limits. Math uses the renderer's existing single-engine
capacity; independent diagrams reserve available CPU capacity for foreground
work and the math engine rather than running every cold source at once.

The remaining parser revealed another verified bottleneck: every unfinished
`\(` or `\[` opener searched the complete remaining message for a missing
closer. The scanner now remembers a failed suffix search independently for
each delimiter. Later openers cannot find a closer that the earlier search
already proved absent. An optimized, std-only extraction of the exact scanner
produced these nine-observation diagnostics:

| Unclosed source | Before p50 / p95 (ms) | After p50 / p95 (ms) |
| --- | ---: | ---: |
| 5,000 parentheses openers, 65 KB | 64.572 / 65.346 | 0.224 / 0.227 |
| 20,000 parentheses openers, 260 KB | 1,034.382 / 1,065.266 | 0.895 / 0.932 |
| 5,000 bracket openers, 65 KB | 64.630 / 65.375 | 0.226 / 0.232 |
| 20,000 bracket openers, 260 KB | 1,043.177 / 1,055.713 | 0.906 / 0.962 |

These scanner-only measurements exclude Markdown parsing/layout and SVG work.
The before/after functions returned identical ranges for 65,536 deterministic
combinations of delimiters, code, escaping and Unicode. New repository
regressions preserve long unfinished text and escaped closers, still find
other math kinds, and keep the first opener when a late closer exists.
Extraction sources and logs are under `target/performance-math-parser-*`.

Regressions also cover composing with a paused cold render, redraw deduplication,
open/closed streaming fences, completed-image reuse, shared sources across
messages, last-reference cleanup, and more live formulas than the global cache.
The optimized chat behavior suite passed all 30 tests (one diagnostic ignored).

Reproduce:

```sh
cargo test -p bruhs-desktop --bin bruhs-desktop --release --locked rich_content_preparation_cost -- --ignored --nocapture
cargo test -p bruhs-desktop --bin bruhs-desktop --release --locked desktop_math_scanner_cost -- --ignored --nocapture
```

## Web transcript and chat update pass

The fifth pass found two independent update costs. Meeting detail cloned its
complete grouped transcript and replaced that subtree after a single segment
change, including every speaker editor. It now projects source records into
stable row signals and keyed speaker runs. Only changed records copy their text;
appending to a run changes its membership without replacing the existing rows.
Speaker label receipts update the affected chip independently of transcript text.
Removed records/runs leave the projection, and reference-counted signals remain
valid until their mounted readers release them. Maps reserve their known input
capacity, and speaker IDs are shared between cache/view copies. Text,
confidence and partial/final updates check the existing run structure in one
borrowed pass and publish changed rows without rebuilding membership.

The separate transcript diagnostic measures cold data preparation and a
single-segment text update at 100, 1,000, 10,000 and 50,000 records. Cold
preparation retains both sides' output until after timing. The former update
includes grouped-memo equality and replacement of the grouped read; the new
update reconciles source records and publishes only the changed row. The source
scan still scales with record count. Neither fixture measures DOM creation,
layout, input-to-paint, IndexedDB, or capture I/O.

The first final run overlapped other work and had large outliers (including
cold p95 up to 59.696 ms); their cause was not isolated. It remains in
`target/performance-web-transcript-projection-release-final.log`. Three further
repetitions of the **unchanged optimized binary**, each with 31 observations,
produced these ranges of per-run p95 values:

| Segments | Former one-row update (ms) | Stable one-row update (ms) | Former cold preparation (ms) | Stable cold preparation (ms) |
| --- | ---: | ---: | ---: | ---: |
| 100 | 0.030–0.033 | 0.005–0.006 | 0.020–0.023 | 0.024–0.038 |
| 1,000 | 0.304–0.347 | 0.040–0.043 | 0.191–0.228 | 0.232–0.247 |
| 10,000 | 3.100–5.670 | 0.368–0.405 | 1.993–2.050 | 2.630–2.701 |
| 50,000 | 15.700–16.051 | 1.934–2.283 | 10.803–11.342 | 15.512–15.842 |

Stable reactive ownership adds cold setup cost in exchange for cheaper repeated
updates and retention of existing editors/text. Cold preparation did not improve;
the larger initial web DOM remains a separate target. These are diagnostic
observations on a busy machine, not tail-latency guarantees. Repetition logs are
`target/performance-web-transcript-projection-repeat-{1,2,3}.log`; their summary
is `target/performance-web-transcript-summary.json`.

Chat also used a sibling scan for every row's memo. A streaming turn with many
parts therefore did quadratic lookup work. Key order and its position index are
now separate memos: text tokens retain the index and the `For` list receives keys
only when membership/order changes, rather than another complete row clone.
Every row still reads its current data by key after reordering or settling.

| Parts in one chat turn | Scan lookup + clone p50 / p95 (ms) | Indexed lookup + clone p50 / p95 (ms) |
| --- | ---: | ---: |
| 100 | 0.022 / 0.026 | 0.031 / 0.033 |
| 1,000 | 1.358 / 1.391 | 0.309 / 0.314 |
| 5,000 | 31.011 / 31.349 | 1.561 / 1.592 |

This is 31 optimized native observations over the actual lookup helper, with a
warm index and one additional working row. It excludes index construction,
Markdown rendering and DOM work. At 100 parts the indexed lookup is slightly
slower; the change removes the large-turn growth and avoids repeated list
construction, rather than claiming a speedup for every small input. The raw
output is `target/performance-web-timeline-lookup-final.log`.

Regressions cover partial-to-final updates, preserved unchanged text allocations
and row identities, appends, deletion, reordering, speaker reassignment, cleanup,
and which reactive consumers recompute. The actual segment-line component's
native HTML serialization preserves complete long text, HTML escaping, partial
opacity and confidence badges. A test-only Tachys SSR feature enables that
serializer without changing the deployed CSR dependency features.

Both web views still mount their complete history. This pass preserves full
text and does not claim to bound DOM size or fix cold browser layout/paint.
The required Expect browser connector is unavailable in this session; no raw
browser fallback was used, and no browser behavior, accessibility, INP or frame
rate acceptance is claimed. Large initial mounts and long streaming Markdown
remain explicit next-loop work.

Reproduce:

```sh
cargo test -p bruhs-web --release --locked transcript_update_cost -- --ignored --nocapture
cargo test -p bruhs-web --release --locked timeline_row_lookup_cost -- --ignored --nocapture
```

## Native palette and mixed-history save pass

The sixth pass found that mixed palette search sorted and constructed every
matching thread pick before the caller took its existing six results. It now
selects and sorts that prefix before copying titles/IDs or formatting ages.
Source position breaks timestamp ties, preserving the former stable ordering.
Matching still examines the same fields and histories; the display limit and
record-search policy are unchanged.

The optimized GPUI diagnostic retains the former implementation for same-binary,
same-fixture comparisons. It uses 31 observations per computation fixture:

| Thread projection fixture | Before p50 / p95 (ms) | After p50 / p95 (ms) |
| --- | ---: | ---: |
| Title match, 100 threads | 0.030 / 0.041 | 0.010 / 0.010 |
| Title match, 1,000 threads | 0.267 / 0.389 | 0.068 / 0.073 |
| Title match, 5,000 threads | 1.506 / 2.443 | 0.410 / 0.556 |
| Body match, 5,000 threads with ~3.6 KB each | 4.235 / 4.772 | 2.846 / 3.100 |
| Absent query, same histories | 2.789 / 3.206 | 2.836 / 3.151 |

These are projection costs, not complete palette frames. An absent query still
scans the same histories; no improvement is attributed to that path. Search
normalization/matching itself was not changed.

With 10,000 indexed local meeting notes, the current input/test-layout fixture
measures **42.682 / 43.298 ms** p50/p95 for `notequartz`, **8.674 / 9.216 ms**
for appending the final `z` to `notequart`, and **6.689 / 7.221 ms** for a Down
selection step. GPUI's `simulate_input` dispatches characters (and empty boundary
tokens) individually, so the whole-input figure is not one physical keystroke.
These include test-driver/layout work and exclude initial index construction,
OS input, GPU presentation and disk completion. They are additional diagnostics,
not a before/after speedup claim for note search or a live latency acceptance.

The expanded snapshot fixture confirms that the existing meeting cache already
shares transcript text. No snapshot ownership/save-queue redesign was made:

| Snapshot fixture | Create + drop p50 / p95 (ms) |
| --- | ---: |
| 100 meetings × 1,000 transcript segments | 0.010 / 0.021 |
| 5,000 knowledge documents with ~8 KB bodies | 3.657 / 4.058 |
| 1,000 documents with two prior versions, 50,000 segments, 5,000 chat messages, 1,000 local notes | 4.235 / 4.525 |

The snapshot fixture creates and drops its owned value on the test/UI thread.
It does not separate construction from destruction, and production transfers
destruction to the worker. Neither serialization nor filesystem work is included.

The background writer now uses compact JSON. Whitespace is not part of the
state schema; existing pretty files and backups still load. The encoding
diagnostic uses 11 observations and includes encoding/output-buffer release,
with no disk I/O:

| Snapshot | Pretty p50 / p95 (ms) | Compact p50 / p95 (ms) | Pretty / compact payload bytes |
| --- | ---: | ---: | ---: |
| Meeting-heavy | 39.921 / 41.005 | 29.632 / 30.229 | 61,530,290 / 48,893,681 |
| Knowledge-heavy | 18.037 / 18.408 | 17.885 / 18.475 | 43,018,314 / 42,565,212 |
| Mixed | 40.698 / 42.108 | 35.134 / 36.772 | 79,398,630 / 71,977,758 |

Large plain-text document bodies dominate the knowledge case; its encoding tail
did not improve. All three payloads shrink, but actual disk/save-completion
latency remains unmeasured. Generation checks, external-writer conflict checks,
atomic replacement, backup bytes, fsync, file permissions, and acknowledged-save
ordering remain intact. A regression writes the former pretty format, reads it,
saves compact state through the production guarded writer, verifies complete
typed content and the exact original backup, then recovers that backup after a
corrupt primary. Palette regressions preserve timestamp ties, per-field query
semantics and keyboard activation. Both new regressions pass in optimized builds.

These are observations on the shared machine, not portable performance limits.
The actual-adapter output is `target/performance-native-actions-final.log`.
Earlier std-only algorithm/JSON-value prototypes are retained separately in
`target/performance-native-prototypes.*`; their timings are not substituted for
the actual-adapter results above.

Reproduce:

```sh
cargo test -p bruhs-desktop --bin bruhs-desktop --release --locked native_action_audit_cost -- --ignored --nocapture
```

## Native calendar preparation and shared lane pass

The seventh pass found that every month cell independently rebuilt and cloned
the entire local/Google event collection. Each merge checked duplicate IDs by
scanning the growing output, then each cell cloned all matching events/tasks
before displaying at most three pills. Task matching also formatted the same
cell date once per task. Week columns repeated the merge and copied day events.

Each native calendar view now merges once. An ID set preserves local precedence,
local duplicates, remote source order and the first valid remote with each ID.
An invalid remote timestamp does not suppress a later valid record. A month
projection visits relevant day spans, borrows the first three events/tasks per
day, and retains complete counts for the existing overflow labels. Dates are
formatted once per grid day; due dates retain exact string matching. Events
still precede tasks within the existing three-pill display policy. Week/day
columns share the prepared collection and filter/clamp it in their existing
layout helper. No records or scheduling actions are removed.

The shared lane allocator now switches from the original linear scan to two
priority queues after more than 64 lanes are required. It still chooses the
lowest available lane and retains stable input order for equal start times.
The switch is an algorithm crossover, not an event/lane limit. A prototype
showed that using queues for ordinary small overlaps was slower; those cases
retain the scan. The optimized actual-function diagnostic uses 31 observations:

| Events / overlap | Before p50 / p95 (ms) | After p50 / p95 (ms) |
| --- | ---: | ---: |
| 1,000 / 1 | 0.002 / 0.002 | 0.002 / 0.002 |
| 1,000 / 8 | 0.003 / 0.004 | 0.003 / 0.004 |
| 1,000 / 1,000 | 0.176 / 0.181 | 0.005 / 0.006 |
| 10,000 / 1 | 0.020 / 0.021 | 0.023 / 0.023 |
| 10,000 / 8 | 0.036 / 0.036 | 0.036 / 0.036 |
| 10,000 / 64 | 0.155 / 0.156 | 0.143 / 0.145 |
| 10,000 / 10,000 | 17.283 / 17.347 | 0.036 / 0.039 |

The few microseconds of overhead in the single-lane 10,000-event case are
retained here; these measurements do not establish a universal speedup.
They include allocation/result destruction and exclude UI rendering. Both
native and web timegrids use this shared allocator. Regression comparisons
cover dense ties, the crossover boundary, lowest-lane reuse, extreme endpoints,
and 2,000 deterministic mixed interval sets, including zero/negative durations.

Native merge/projection regressions compare the former behavior, including
midnight boundaries, multiday/all-day events, noncanonical due strings, counts,
source ordering and exact event fields. Existing GPUI calendar tests exercise
mode/navigation/create actions, pill/block clicks, and moving/resizing events.
These are test-layout/state checks; real OS input, GPU presentation, browser
layout and multi-calendar network sync remain separate measurements.

The native diagnostic uses the current renderer for both modes, with test-only
adapters retaining the former preparation paths. Both modes use the new shared
lane allocator, so it isolates native preparation rather than claiming a second
lane-allocation speedup. The month legacy adapter adds a small borrowed-view
adapter after its original work. Events are distributed across 180 days and
tasks across 84 due dates; the total dataset is not the number of mounted pills.
Persistence is disabled in these test fixtures. Optimized measurements are
diagnostic observations on this shared machine, not latency thresholds or INP.

At a 1,800 × 1,000 test viewport, redraws use 11 observations after a warmup.
Navigation uses 12 alternating next/previous actions, including state update
and test layout. The reported p95 is the largest observation at these sample
counts. “Each” means that many local events, Google events and tasks, with
roughly 1 KB event notes and 2 KB task descriptions:

| Native action / records of each kind | Before p50 / p95 (ms) | After p50 / p95 (ms) |
| --- | ---: | ---: |
| Month redraw / 100 each | 8.514 / 8.847 | 5.632 / 5.880 |
| Month redraw / 1,000 each | 101.456 / 103.257 | 7.742 / 8.237 |
| Month redraw / 5,000 each | 1,875.291 / 2,021.303 | 10.270 / 11.827 |
| Week redraw / 1,000 each | 19.349 / 20.207 | 4.386 / 4.799 |
| Day redraw / 1,000 each | 3.170 / 3.734 | 1.222 / 1.726 |
| Month navigation / 1,000 each | 102.435 / 108.694 | 7.746 / 7.975 |
| Week navigation / 1,000 each | 19.047 / 20.588 | 4.427 / 5.205 |
| Day navigation / 1,000 each | 3.144 / 3.934 | 1.235 / 1.574 |

No observations were excluded; the reported tails include the slower old path.
The measurement does not include disk completion, OS dispatch or GPU present.
Both new merge/projection regressions also pass in the optimized binary.

Reproduce:

```sh
cargo test -p bruhs-models --release --locked dense_calendar_lane_cost -- --ignored --nocapture
cargo test -p bruhs-desktop --bin bruhs-desktop --release --locked dense_calendar_action_cost -- --ignored --nocapture
```

Raw lane measurements are in `target/performance-calendar-lanes-release.log`;
native measurements are in `target/performance-calendar-native-release.log`.
The earlier metadata/queue prototypes are retained separately under
`target/performance-calendar-*-prototype.*` and are not substituted for these
actual-adapter/function results.

## Browser streaming Markdown pass

The eighth pass measures the production WASM app in a disposable browser
origin. The pre-change app already retains the `.prose-msg` element, but its
String `inner_html` update replaces all children after every text event.
Browser observations confirm that the first paragraph is replaced on every
sample, and forced layout dominates the long multiparagraph fixture.

Assistant prose now uses a typed sanitized HTML value whose initial build and
HTML serialization retain the existing behavior. Rebuilds reconcile the
complete sanitized HTML tree, preserving equal nodes, updating changed
attributes/children, and appending to growing text nodes with `appendData`.
The retained state has the same element/HTML ownership as Tachys's ordinary
String attribute; there is no new history-wide DOM cache. Changed content
clears the render-pass marker so settled highlighting, math and diagrams still
run. The full Markdown parser and URL/HTML policy remain the source of the
desired tree, including earlier output changed by later reference definitions.

A separate browser trace held the synthetic stream's EOF after its Result.
The selected prose stayed attached after Result processing, then the whole
turn was removed when EOF published the reply into history. Renaming the same
thread also remounted its transcript. The chat's outer view read the complete
thread metadata, and its inner view read message updates to choose between
the empty hero and transcript; unchanged branch results still recreated the
component owners. `ChatMount` now memoizes the selected thread's identity and
presence, plus the hero/transcript boolean. Detailed updates continue through
the mounted header and timeline. Two targeted regressions verify stable
consumers during metadata edits/completion and correct navigation, deletion,
and empty-to-first-message transitions.

The packaged JavaScript helper passes 32 browser contract checks covering structural
changes, attributes, tables, tasks, code, footnotes, Unicode, deletion, marker
invalidation, stable prefix nodes and selections in earlier/growing text.
The reusable browser fixture is `bruhs-web/tests/markdown_dom_contracts.mjs`;
run its exported function in a disposable browser with the packaged helper
and that browser's document. It is a browser fixture, not a Node DOM emulator.
The integrated behavior fixture streams twelve successive prefixes, including
late link/footnote definitions, partial inline code/fences, a table, unsafe
markup/URLs, task lists, math and Mermaid. All twelve final HTML/text hashes,
links, footnote numbers and checkbox counts match the baseline. The selected
prefix now survives all twelve updates and completion. Two math expressions,
one diagram and one highlighted code block render successfully and survive
renaming the thread. No injected elements or external browser resource
requests occur. Error and Stop checks retain the complete partial reply and
restore the idle composer; the expected provider-error message remains visible.

Both builds use `trunk build --release --locked`, with the same disposable
synthetic HTTP/SSE server and the actual client decoder, reactive rows, CSS and
fonts. No provider runs, audio capture, real workspace data or external service
changes are involved. Each fixture receives a complete prefix, waits for fonts,
then measures 21 sequential text updates. The first full draw is recorded
separately. SHA-256 hashes cover the complete DOM HTML and text; the timing
fixture also verifies settled highlighting preserves the complete text.

The clock starts when an agent-stream chunk becomes available to the client
and records the DOM mutation, followed by an explicit geometry read that
flushes layout. The result includes decoder/reactive/rendering work and forced
layout, excluding the control request's round trip. It is not keystroke INP or
GPU/presentation latency. The T3 Chromium 152/Electron 44 preview reports
`visibilityState=visible`, but two animation frames took about 996 ms;
presentation cadence is a tool/platform limitation in this fixture. The
primary runs yield an event-loop task between updates instead. Earlier
animation-frame-paced observations remain separate in
`target/performance-browser-baseline.json`.

The final production build measures the following. Each row has 21 updates;
p95 is the twentieth ordered observation. No observations were excluded:

| Stream fixture / UTF-8 prefix bytes | Before p50 / p95 (ms) | Final p50 / p95 (ms) |
| --- | ---: | ---: |
| Short prose / 342 | 3.3 / 3.9 | 0.2 / 0.5 |
| Multiple paragraphs / 11,360 | 101.7 / 104.8 | 0.7 / 1.3 |
| Multiple paragraphs / 113,421 | 1,057.4 / 1,080.3 | 5.6 / 10.7 |
| Code block / 102,528 | 6.3 / 8.8 | 3.3 / 9.2 |
| One long paragraph / 96,025 | 19.9 / 20.5 | 8.8 / 10.3 |

Reconciliation adds some DOM-processing work. Code's DOM-phase p95 rises from
0.8 to 2.2 ms; the long paragraph's rises from 4.5 to 7.4 ms. Reduced layout
work improves their medians and the long paragraph's total tail, but the
primary code-block p95 is 0.4 ms worse. Two unchanged-final-build code repeats
record p95 **7.0** and **7.4 ms**, including a **28.6 ms** maximum observation.
All repeats and tails are retained in `target/performance-browser-code-repeats.json`.
No uniform code-tail speedup or portable latency limit is claimed. Full parsing
and initial mounting still need further work; the first complete draw is not
used to claim a speedup here.

Across all five primary replays, complete HTML/text and settled text hashes
match, and the original first block stays attached for **105 / 105** updates
(the baseline retained it for zero). The long multiparagraph fixture's forced
layout p95 falls from **1,068.5 to 0.9 ms**; its remaining DOM-phase p95 is
**9.9 ms**, including complete parsing and reconciliation.

Separate controlled actions on approximately 113 KB of prose isolate the
mounting fix. Their before build already contains DOM reconciliation. These
are **one observed trial per action**, not percentile estimates:

| Controlled action | Before (ms) | Final (ms) |
| --- | ---: | ---: |
| Held EOF release → settled layout | 1,106.9 | 14.3 |
| Rename Enter dispatch → layout | 1,301.8 | 1.1 |

Both actions retain the same turn, prose and first paragraph in the final
build, with complete text preserved. The completion selection survives.
The rich-content rename check also retains its math/diagram/highlight nodes.
One synthetic Stop observation takes 55.9 ms from click dispatch to settled
layout; it establishes behavior and records the transport-abort path, not
live provider cancellation latency or a Stop speedup.

The primary baseline is `target/performance-browser-baseline-fast.json`;
the final raw replay is `target/performance-browser-after-final.json`, with
derived comparisons in `target/performance-browser-comparison-final.json`.
`target/performance-browser-acceptance.json` records the combined checks.
The replay scripts, server and probe are under
`target/performance-browser-{fixture.py,probe.js,run.js,adversarial.js,lifecycle.js,rename.js}`.
The frozen pre-change production assets are in
`target/performance-browser-before/dist/`; the final build is isolated in
`target/performance-browser-final/dist/`. Build hashes are recorded in
`target/performance-browser-builds.json`. Earlier partial-fix runs remain
separate under `performance-browser-after*` and are not substituted for the
final measurements. A fixture keep-alive routing error during asset switching
was corrected before accepted integrated replays. This pass's baseline includes
the previous performance fixes and concurrent workspace changes.

## Findings fixed

| Severity | Path | Finding and change |
| --- | --- | --- |
| High | Web/native thread navigation and shelves | Each ordered ID triggered a full thread scan. Both adapters now resolve IDs through one index; the native workspace also indexes streaming/approval lookups. |
| High | Shared chat/diff find | Case-insensitive comparisons allocated lowercase character vectors for mismatches. ASCII comparisons and Unicode lowercase iterators now allocate nothing per comparison; query characters are prepared once. Unicode scalar offsets and non-overlapping matches are preserved. |
| High | Web chat find | Closed/empty find still prepared transcript strings. Find now exits before touching history; settled searchable text is memoized independently of the live message and query. |
| Medium | Web/native diff find | Find cloned full files and line strings. It now searches borrowed line text and skips empty queries; the web also skips closed find. |
| Medium | Universal search | Every match was sorted before returning a small result set. Partition first, then sort the returned results, preserving score/recency/source-order ties. |
| Medium | Plan my day | Every eligible task was sorted although only the first 50 can be shown. Select and sort those 50 while preserving the total eligible/unscheduled count and six-block scheduling policy. |
| High | Shared edit diff | Equal prefixes occupied a quadratic LCS table. Emit that guaranteed context first, retaining the existing large-input fallback and removal tie-breaking. |
| High | Server upload/viewer/send | Attachment writes, preview reads, upload-path validation, and pasted-text writes blocked async request workers. They now run in `spawn_blocking`; attachment-free sends skip the work. |
| High | Native send/steer | Pasted text and checkpoint path resolution ran on the UI thread. Prepare them on workers before constructing/sending the provider turn. Plain sends without attachments/worktrees retain the direct path. |
| High | Web/native new terminal | PTY creation, directory checks, and shell spawning ran on the caller's worker/UI thread. Launch on background workers. Native late results cannot steal selection from a newer launch, and unadopted sessions are closed. |
| Medium | Files panel | Every thread metadata change cleared/refetched the active directory, including unrelated thread changes. A memoized project identity now drives fetches. |
| Medium | Startup | Capture status and calendar hydration waited for people hydration. Independent startup operations now begin independently; people refresh still waits for its local hydration. |
| Medium | Update checks | HTTP version checks constructed a client per request. Reuse the process client and its connection pool with the existing timeout. |
| High | Server skill picker | `$` lookup synchronously scanned skill folders on an async request worker. Offload that scan; native lookup already ran off the UI thread. |
| Medium | Open in editor | PATH/file checks and process spawning blocked the server worker/native UI. Run them on a worker and retain the same errors and native notice. |
| High | Native task list/board | Every frame cloned complete task records and mounted all rows/cards; the board repeated filtering/cloning for each column. Filter borrowed tasks once, group borrowed references, and render pages matching the existing web budgets (100 list rows, 40 cards per column). Selection, project filters, and status moves reveal the appropriate page; ordinary text edits preserve manual paging. |
| High | Native knowledge library | Every document/source was rendered on each frame. Both sections now independently page 24 cards, matching the web library budget, with complete record text retained in the detail editors. |
| High | Native chat history | Every message was constructed and laid out on each redraw. A variable-height GPUI list now renders only the viewport, keeps geometry separate from messages, and searches the full history. Home/End, arrow and page keys navigate the transcript. New sends follow the tail; incoming tokens preserve a reader's position in older history. The previously rendered range is remeasured to clamp the scroll anchor when rows shrink. |
| High | Native cold math/diagrams | Cache misses synchronously generated SVG on the UI thread. Workers now generate/hash images with exact-source deduplication, selectable pending source, and cancellation/cleanup when content changes. |
| High | Native unfinished math | Each unclosed parenthesis/bracket math opener searched the rest of the message again. Remember absent closing delimiters for the current parse, preserving source text and independent delimiter handling. |
| High | Web meeting transcript updates | Every segment update cloned/replaced the complete transcript subtree. Stable record signals and keyed speaker runs now update changed text and membership independently; metadata label receipts do not replace transcript text. |
| High | Web chat row lookup | Every row memo scanned all sibling rows and the keyed list read another full row clone. Separate key/index memos resolve current rows directly and retain list membership during text-only updates. |
| Medium | Native palette thread picks | Every matching thread was sorted and copied before six picks were retained. Select/sort the existing prefix before constructing picks; preserve recency and source-order ties. |
| Medium | Native snapshot encoding | Pretty JSON added worker CPU and payload bytes to private state writes. Compact encoding preserves the schema and old-file/backup compatibility while retaining the save protocol. |
| High | Native calendar preparation | Each month cell repeated a full event merge with linear ID deduplication, cloned hidden records, and formatted dates per task. Merge once per view, index IDs, and prepare borrowed month picks/counts; week/day columns share the prepared events. |
| High | Shared calendar overlap layout | Each fully overlapping event scanned every occupied lane. Retain the ordinary small-lane scan and switch large sets to indexed end-time/available-lane queues, preserving stable ties and the lowest-free-lane policy. |
| High | Web streaming Markdown | Every text event replaced all prose children, invalidating unchanged paragraph layout and selection. Reconcile complete sanitized HTML, retain equal nodes, append growing text in place, and invalidate settled render markers only when content changes. |
| High | Web chat mounting | Same-thread metadata edits and nonempty history updates recreated transcript owners. Memoize conversation identity/presence and empty-state transitions; keep detailed updates in mounted children. Completion and rename now retain the transcript and rich-content nodes. |

No durability checks, permission checks, data validation, retry ordering, or
existing runtime limits were removed to improve these numbers. The existing
50-item planning presentation policy and result limits were retained.

## Action coverage

“Reviewed” below means the action's dispatch, expensive operations, and shared
helpers were inspected. It does not imply a live browser/native latency
measurement. Existing tests cover many more behavioral cases than these
performance fixtures.

| Action family | Dispatch/data path reviewed | Result / next measurement |
| --- | --- | --- |
| Navigate Today, Chat, Inbox, Knowledge, Tasks, Calendar, Life, Automations, Meetings, People, Search, Terminal, Settings | Web command bar/shell, native palette/workspace, view routing | Navigation is local state. Thread-order computation fixed; route-mount/paint timing remains. |
| Open project; pick/recent project; create/new thread; import thread | Web project/import dialogs and thread catalog; native project picker/import/threads; server import/paths | Disk imports and native picker resolution already use workers. Provider-history discovery still needs large-fixture timing. |
| Pin, settle, snooze, wake, drag/reorder, delete, undo; previous/next/jump thread | Shared sidebar rules, web sidebar, native sidebar/threads | Repeated ID scans fixed; existing grouping, folding, rank and selection tests retained. |
| Copy thread link/resume command/Markdown; continue in T3; copy pull link/number | Web command bar/thread export; native palette/threads/import; core import/export | Local formatting/clipboard or worker-based export. Large transcript export needs a timing fixture. |
| Compose, mention files/threads/skills/pulls, stash, recall, model/permission/effort selection | Web composer/mention editor; native hints/composer; core files/skills/pulls | Files are indexed with an existing TTL; network hints are asynchronous. Server skills scan offloaded. Composer input-to-paint and cold file-index contention remain. |
| Send, steer, queue, stop, retry, rewind, compact, handoff, goals, permissions, secrets, second opinion | Web chat/stream/API decoder; native chat/streaming; core orchestrator/registry/secrets | File preparation offloaded. Settled/live projection remains separate. Browser synthetic streaming, completion, error and Stop retain complete prose; stable mount boundaries avoid rebuilding the conversation during completion. Live CLI/provider latency, queue/cancellation races and additional interactions remain runtime targets. |
| Find, expand thinking/tool output, changed files, view diff, mark reviewed, revert | Shared find/diff/review; web timeline/diff; native chat/diff | Find/diff hot paths fixed and benchmarked. Native chat virtualizes messages and prepares cold SVG on workers. Web prose now retains unchanged DOM/selection across updates and completion; complete-output and rich-content checks pass. Initial web history mounting, code-block tail overhead, native long prose, reviewed-state persistence and physical paint remain. |
| Toggle sidebar, right panel, diff, tasks, preview, files, terminal; resize | Web shell/right panel; native workspace/panels; shared panel state | Immediate local toggles; FilesPanel refetch fixed. Frame timing during drag/resize remains. |
| New/close/select terminal, type, resize, replay/reconnect, cite output; open system terminal | Core terminal; web terminal glue; server WS; native terminal panel/view | PTY launch offloaded. Late selection/close races and deferred focus guarded; error-path regressions pass. Existing dedicated PTY reader/writer/reaper and atomic replay/stream handoff retained. Successful concurrent-open lifecycle needs more cases. |
| Browse/open attached/project files; upload/drop/paste/remove/retry attachment; open editor | Core files/uploads/editor; server file/upload/editor routes; native files/panels | Blocking I/O offloaded; preview identity guards retained. Slow-filesystem and maximum-sized attachment timing remains. |
| Local preview discovery/open; pull refresh/open | Core preview/pulls; server routes; web/native panels | Async bounded-deadline child processes already used. No live external requests performed for timing. |
| Task/project create/edit/delete; status/priority/due/dependency/recurrence; board drag/list/selection; day plan apply/undo | Shared planning; web task stores/list/board; native tasks/today/scheduling | Planning fixed. Native task list/board now page borrowed records using the existing web budgets. Pointer/keyboard paging, full descriptions, selection across modes/status moves, manual paging during edits, and project-filter clamping pass. Ordered, original-checked saves retained. |
| Calendar prev/next/today/month/week/day; create/edit/delete/drag/resize; scheduling links/book; connect/sync/disconnect Google | Shared calendar/scheduling; web month/timegrid/booking; native calendar; core Google | Native preparation now merges once, indexes duplicate IDs, and borrows month picks while retaining counts. Shared dense-lane allocation is indexed above the measured crossover. Optimized computation/test-layout fixtures and native click/move/resize regressions pass. Google uses pooled HTTP and per-calendar incremental cursors. Real GPU/browser interaction latency and multi-calendar sync remain. |
| Meeting record/stop/open; transcript copy/delete; speakers/link/rename; summarize/re-extract/retry/cancel; notes; dock/meter | Web meeting actions/transcript sync; native hub/cache/transcript list; server pump/SQLite; capture pipeline | Redundant preparation copies removed; stable web rows and speaker runs avoid full subtree replacement after a segment change. Data identities, reactive invalidation and complete line serialization pass regressions. Capture/model/SQLite work already uses workers with recovery/performance tests. Initial web DOM/layout, real input/selection behavior and audio throughput remain. Concurrent transcription-provider work is outside these perf edits. |
| People CRUD/merge/reminders/relationships/voice enrollment | Web people directory/queues; native people/cache/merge; core SQLite/contacts | Existing indexed contact lookups, compact directory pages, atomic merges, durable queues and database-worker responsiveness tests retained. Large-profile UI timing remains. |
| Inbox import/open/read/archive/snooze/star/delete/draft/send/ask agent | Shared inbox; web inbox page; native inbox | Full-history library clones replaced by compact paged cards; web search indexes full text once. All-page reachability, Unicode/deep-message search, triage counts, pointer navigation, detail history and stale-page clamping regressions pass. Live browser paint and mutation-to-paint remain. |
| Knowledge documents/clips/attachments/backlinks; search/pick/link/delete; capture conversion | Shared knowledge; web indexed paged library/record picker; native knowledge/storage; workflow transaction | Shared backlinks/mutation benchmark runs. Both libraries now bound mounted cards; independent native document/source pages and complete detail text pass pointer regressions. Real GPU/browser paint and large detail editors remain. |
| Finance/health CRUD/import/habits/insights/providers; automation preview/approve/run/cancel/retry/rollback/audit | Shared life/automation; web workflow stores; native life/automations/MCP bridge | Domain benchmarks and transaction/recovery tests retained. Metadata serialization/snapshot and large audit history rendering remain. |
| Settings/keybindings/auth/login/update checks/provider update/app update | Web settings/auth/updates; native settings/auth/updates; core updates | Version HTTP pool fixed; existing async commands, timeouts and detached installer queues retained. CLI installation/update actions were not triggered. |
| Landing page palette, surface tabs/tour, download links | `bruhs-site/public/site.js`, `palette.js`, static assets | Small local command set, incremental keyboard selection and offscreen/hidden tour pause already present. User's landing-page edits retained; site tests included in gate. |
| Save/reload/offline/recovery/MCP reads and writes | Web IndexedDB/task saves/workflow; native save queue/state file; core workspace bridge/SQLite | Atomicity and visible recovery retained. Chat-heavy, meeting-heavy, knowledge-heavy and mixed snapshot create/drop fixtures are measured. Compact native encoding reduces payloads; legacy primary/backup compatibility and recovery pass. Actual disk completion, additional history mixes and large IndexedDB writes remain targets. |

## Validation

The first targeted core/server/web regression run passed (360 core unit tests,
118 server unit tests, 379 web unit tests, plus integration/doc tests). Subsequent
workspace checks include concurrent transcription changes as well as these edits.
`scripts/gate.sh --quick` passed: formatting, workspace/native Clippy,
WASM Clippy, **2,223 Rust tests passed, zero failed, seven ignored**, the
15 landing-page tests, site type checking, and benchmark fixture self-checks.
After the final worker changes and native send sequencing review,
`cargo test --workspace --locked` and workspace Clippy with `-D warnings`
both passed again. `cargo build -p bruhs-web --target wasm32-unknown-unknown
--locked` passed. Formatting and whitespace checks passed.

Added behavioral checks cover bounded ranking ties, Unicode case/offset behavior,
LCS line numbering/ties, planning beyond the visible 50 tasks, server file-viewer
and upload-path contracts, and asynchronous native terminal launch failure.
The timing fixtures deliberately have no machine-specific pass/fail thresholds.

Second-pass workspace tests passed **2,231 tests, zero failed, nine ignored**
before the terminal selection/close follow-up. That follow-up regression also
passed separately. Workspace Clippy passed at that source snapshot. WASM Clippy,
the WASM build, formatting of the performance-edited Rust files, and whitespace
checks passed after the changes. The optimized native diagnostic passed too.
The latest quick gate passed WASM Clippy, all 15 site tests, site type checking,
and benchmark self-checks, but its full-workspace format/Clippy/test steps ran
during concurrent service-connection implementation and failed on missing
`bruhs-core/src/connections` modules. This is not a current all-green gate claim;
rerun it when those files are complete. That concurrent implementation was not
reverted or replaced by the performance pass.

Third-pass native regressions passed **601 tests**, including the new paging,
offscreen find, keyboard history, streaming position and shrinking-row cases.
The full workspace run subsequently passed **2,255 tests, zero failed, ten
ignored**, plus **21 site tests**, site type checking and benchmark self-checks.
Formatting, whitespace and WASM Clippy passed. An initial accessibility failure
on the new transcript focus outline was fixed and its audit now passes.
At the third-pass snapshot the full gate reported Clippy failures for resource lifetimes in
the concurrently changing core service-connection implementation and existing
core tests. Earlier workspace Clippy passed before those additional edits.
That pass did not claim an all-green gate at that point.

The fourth-pass final quick gate is **all green**: formatting, workspace/native
Clippy with `-D warnings`, WASM Clippy, **2,266 Rust tests passed, zero failed,
12 ignored**, **33 site tests**, site type checking and benchmark self-checks.
This run includes the unfinished-math parser fix and supersedes the earlier
concurrent-module/Clippy failures above. Logs are isolated under
`target/performance-gate-pass4-parser/`; the exact gate script was copied to
`target/` with only its log destination changed to avoid other work overwriting
the results. Whitespace checks pass. The optimized rich-content diagnostic and
all 30 optimized chat behavior tests also pass; the parser's two new large-input
regressions pass in the final workspace run. No live browser or GPU performance
claim is made.

The fifth-pass final quick gate also passes in full: formatting, workspace/native
and WASM Clippy, **2,273 Rust tests passed, zero failed, 14 ignored**, **33 site
tests**, site type checking and benchmark self-checks. This run includes the
final transcript structure fast path and indexed chat rows. Its isolated step
logs are `target/performance-gate-pass5-structure/`. The optimized lookup and
transcript diagnostics pass, including all three unchanged-binary transcript
repetitions. Whitespace checks pass. The seven added behavioral/serialization
tests establish data/identity/reactivity contracts, not live browser acceptance.

The sixth-pass final quick gate passes formatting, workspace/native and WASM
Clippy, **2,303 Rust tests, zero failed, 15 ignored**, **33 site tests**, site
type checking and benchmark self-checks. The run includes concurrent thread-link
implementation as well as these performance changes; its additional tests are
not all attributed to this pass. Isolated logs are under
`target/performance-gate-pass6-final/`. The optimized actual-adapter diagnostic
and both new optimized compatibility/keyboard regressions pass. Whitespace
checks pass. No GPU, OS input, browser or live external-provider acceptance is
claimed.

The seventh-pass quick gate passes formatting, workspace/native and WASM
Clippy, **2,323 Rust tests, zero failed, 17 ignored**, **33 site tests**, site
type checking and benchmark self-checks. Logs are isolated under
`target/performance-gate-pass7/`, with the summary in
`target/performance-calendar-gate.log`. The run includes concurrent workspace
changes; not all additional tests belong to the performance pass. The focused
calendar suite also passes, including existing native pointer/state flows.
Optimized lane/native diagnostics and both new optimized merge/projection
regressions pass. Whitespace checks pass. No OS input, GPU,
browser or live external-provider acceptance is claimed by these tests.

The eighth-pass final quick gate passes formatting, workspace/native and WASM
Clippy, **2,325 Rust tests, zero failed, 17 ignored**, **33 site tests**, site
type checking and benchmark self-checks. Logs are isolated in
`target/performance-gate-pass8-identity/`, with the summary at
`target/performance-streaming-gate-identity.log`. The final production Trunk
build passes. The two mount-state regressions pass; JavaScript syntax checks
and the 32 persistent browser contracts pass against the packaged module.
All five primary browser streaming replays, twelve contextual Markdown
prefixes, completion/rename identity checks, rich-content retention, expected
error visibility and Stop behavior pass. Current browser JavaScript errors and
external browser resource requests are zero. Whitespace checks pass.
These results establish the stated synthetic browser computation/layout and
behavior contracts; GPU paint, OS input, real provider/capture and a universal
all-actions latency acceptance remain open.

## Web history layout and font fallback pass

The ninth pass fixes emoji font fallback and an extra document viewport created
by Mermaid's hidden tooltip. It keeps eager layout and the complete history DOM.
The tested offscreen-containment prototype is **not shipped**.

The typography roles and default Neon stack now name local emoji fonts after
their existing monospace fallbacks. Ordinary text keeps its Monaspace face;
there are no additional font downloads. Adding these fallback families alone
reduced the 1,400-paragraph pilot's full layout from roughly one second to
38–44 ms without skipping any paragraphs. This isolates a font-stack cost;
repeated system fallback work is the inferred mechanism, not an OS font trace.

The blanket `body > div { height: 100%; }` rule also gave Mermaid's floating
helper an 800 px height. The app root already uses `h-full`. Removing that
blanket rule and positioning the idle helper at the origin keeps the document
at 800 px with an unchanged 800 px app root; the helper retains its natural
6 px height. Mermaid's explicit inline hover positioning still takes precedence.
This is a geometry correction, not a measured speedup.

### Current production comparison

Both variants use the **same current production WASM**, SHA-256
`7fb41b5c4eae584167f90625b17b46ce6ed166caa6d07eb7767c628dadba6470`
(12,758,272 bytes). Only the original versus final typography/layout stylesheet
differs; all remaining HTML and assets are identical. This isolates these CSS
changes from concurrent Rust work. The final stylesheet matches
`bruhs-web/index.html` exactly. The final build is
`target/performance-browser-history-font-final/dist`; its comparison baseline
is `target/performance-browser-history-before/dist`.

Measurements use the existing saved synthetic histories, loaded fonts,
Chromium 152 / Electron 44 on this shared macOS machine, and a 1280 × 800 CSS
viewport. Each observation switches through the same short thread or meeting
list and opens the requested history. The clock runs from click dispatch
through the first complete DOM and an explicit layout flush. Transcript
results are warm selected-meeting opens; the first cache refresh is retained
separately. Code observations stop at the first complete prose DOM, before
postprocessing finishes; complete settled content is checked afterward.
These are CPU/layout diagnostics, not paint, OS input, INP, cold WASM startup,
highlight-ready, live provider, or capture latency. Percentiles use nearest
rank. Other workspace activity was possible; small changes in control fixtures
are not attributed to this fix.

| Warm opening fixture | Observations per variant | Before p50 / p95 (ms) | Final p50 / p95 (ms) |
| --- | ---: | ---: | ---: |
| Chat, 1,400 emoji paragraphs | 21 | 1042.1 / 1065.6 | 33.3 / 36.3 |
| Chat, 4 emoji paragraphs | 7 | 12.1 / 14.4 | 4.4 / 6.2 |
| Chat, 140 emoji paragraphs | 7 | 109.6 / 114.8 | 7.4 / 8.8 |
| Chat, 2,500 code lines | 7 | 12.6 / 14.8 | 10.3 / 12.2 |
| Chat, one long ASCII paragraph | 7 | 27.4 / 29.1 | 22.8 / 23.7 |
| Transcript, 100 segments | 7 | 4.7 / 8.8 | 3.4 / 3.5 |
| Transcript, 1,000 segments | 7 | 24.1 / 36.4 | 20.8 / 21.3 |
| Transcript, 10,000 segments | 21 | 196.0 / 209.2 | 196.3 / 203.4 |

All five chat fixtures retain identical complete HTML and text hashes and
identical final heights. All three transcript fixtures retain their complete
row counts and text hashes. The 10,000-segment transcript's median is essentially
unchanged; its full DOM construction and layout still need work. Code and ASCII
controls are recorded without an attributed algorithmic speedup.

### Selection rejected the containment prototype

`content-visibility: auto` with remembered intrinsic heights made initial
opening substantially cheaper and passed the full-text, search, keyboard,
selection, accessibility-tree and reader-position checks. It deferred layout
onto whole-history selection, however. On the same current WASM, selecting the
complete 1,400-paragraph reply rose from **6.0 to 34.1 ms**, and selecting the
complete 10,000-segment transcript rose from **47.1 to 184.3 ms**. These are
single cold-selection CPU observations, not percentile estimates.

The earlier containment-only, original-font prototype exposed an even larger
selection cost of **998.6 ms**. Explicit emoji fallback reduced it, but the
remaining deferred-work tradeoff was enough to reject containment. Neither
`content-visibility` nor intrinsic-height estimates remain in the final source.

The final eager-layout version selects the complete reply in **5.8 ms** and
the complete transcript in **46.3 ms** in the same single-observation check.
Raw selected text is complete in every variant. The final change therefore
does not depend on postponing the layout cost to selection.

### Final behavior and validation

The current quick gate passes formatting, workspace and WASM Clippy with
`-D warnings`, **2,330 Rust tests, zero failed, 17 ignored**, **36 site tests**,
site type checking and benchmark self-checks. This includes concurrent work;
the additional tests are not all attributed to this performance pass. Logs
are isolated under `target/performance-gate-pass9/`, with the summary in
`target/performance-history-gate.log`. The final production Trunk build passes
(`target/performance-browser-history-font-final-build.log`), and whitespace
checks pass.

One intermediate rebuild reported an incompatible dither API while other
workspace builds were active. The source API was compatible when re-read;
rebuilding dependency metadata and then the current application succeeded.
No dither or braille source content was changed. The failed and successful
logs are retained as `performance-browser-containment-root-build.log` and
`performance-browser-containment-root-retry-build.log`.

Final browser acceptance covers complete copy payloads, find, deep transcript
reveal/selection, responsive layout, rich-content retention and streaming reader
position. The packaged Markdown reconciliation module again passes all
**32 browser contracts**. Detailed browser results and the last current-page
diagnostics are retained under `target/performance-browser-history-*.json`.

The Copy response action delivers all **113,600 bytes of original Markdown**
to a mocked clipboard writer; the real clipboard is preserved. This verifies
the application payload, not OS clipboard completion. Find reports the expected
single saved-history match. The last of 10,000 transcript rows remains complete
and visible at a 390 × 844 CSS viewport, with no document horizontal overflow.
The rich fixture retains 220 wrapped paragraphs, 1,870 links, math, a Mermaid
diagram and highlighted code. During the final synthetic SSE check, selecting a
paragraph while 200 more paragraphs arrive and the turn completes retains its
DOM nodes, text selection and exact screen/scroll position (zero-pixel delta).
Returning to the end reaches the true bottom. Rich nodes survive both updates
and completion. The settled composer is idle, the fixture has no pending agent
events, and JavaScript errors and external browser resource requests are zero.

The fixture now returns the requested update-check `online` flag. Its previous
mismatched response caused synthetic retries; both variants of this pass use
the corrected fixture. Earlier raw results remain retained; times across
different fixture versions should not be treated as an isolated comparison.

Reproduce with the disposable fixture server on port 49466 and the matching
`BRUHS_API_URL`. Set `performance-browser-current.txt` to each manifest's dist,
load the page, then install `performance-browser-mount-probe.js`,
`performance-browser-transcript-mount.js` and
`performance-browser-history-suite.js`. Launch `__launchHistorySuite(label)`
without awaiting the entire run in one preview call and collect its completed
named job. Authoritative inputs/results are
`performance-browser-history-before.json`,
`performance-browser-history-font-after.json`,
`performance-browser-history-font-builds.json` and
`performance-browser-history-comparison.json`. Prototypes, mixed tails and
rejected containment replays remain separate and are not substituted for the
final implementation.

The volume filled while saving the chart after validation. Only obsolete
prototype distributions were removed; the final comparison builds, sources,
raw measurements, manifests and logs remain. The oversized synthetic HTTP log
was losslessly archived and verified as `performance-browser-fixture.log.gz`.
Free space remains limited; the completed build and gate results precede this
cleanup.

## Web transcript font shaping pass

The tenth pass removes optional Neon font shaping from saved transcript prose.
Profiling the 10,000-row fixture separated roughly 60 ms of DOM construction
from roughly 135 ms of forced layout. Minimum-width, wrapping and layout/style
containment pilots did not remove that cost. Disabling optional contextual
alternates and ligatures did, while retaining the same Neon font file and every
transcript row.

An element-wide `font-variant-ligatures: none` prototype changed the Arabic
sample's width from 143.7 to 168.9 px and was rejected. The implementation adds
a `MonaspaceNeonTranscript` face using the existing local Neon URL, with
`font-feature-settings: 'calt' 0, 'liga' 0` on that face only. The
`[data-transcript-segment]` selector chooses it; other Neon roles retain their
usual features, and fallback fonts retain their own shaping defaults. The
distinction follows the scope of font-face descriptors in
[CSS Fonts 4](https://www.w3.org/TR/css-fonts-4/#descdef-font-face-font-feature-settings).
No layout is skipped or deferred, and the complete transcript remains mounted.

### Controlled production comparison

Both distributions use identical current production WASM, SHA-256
`0d3dae4b9303a986bb52b28e50078eeec222249d9a9a1df1e424d6be33c4178a`.
Only the typography/layout stylesheet differs. The baseline contains the
previous pass's accepted stylesheet; the final one matches the current source.
This isolates the CSS change from concurrent workspace Rust changes. Builds are
retained in `target/performance-browser-transcript-before/dist` and
`target/performance-browser-transcript-face/dist`, with their manifest in
`target/performance-transcript-face-builds.json`.

Measurements use Chromium 152 / Electron 44 at a 1280 × 800 CSS viewport on the
same shared macOS machine. Fonts are ready before each click. The clock runs
from opening the meeting through its complete row DOM and an explicit layout
flush. There are 63 accepted warm observations per variant: 21 for 10,000 rows
and seven for each other fixture. Percentiles use nearest rank, so the
seven-observation p95 is the maximum. These are computation/layout diagnostics,
excluding physical paint, OS input, provider/capture work, and completion of
the asynchronous IndexedDB save. They establish neither INP nor an all-actions
latency target.

| Warm transcript opening | Rows | Before p50 / p95 (ms) | Final p50 / p95 (ms) |
| --- | ---: | ---: | ---: |
| Plain saved transcript | 100 | 5.0 / 6.1 | 4.5 / 7.1 |
| Plain saved transcript | 1,000 | 21.8 / 26.5 | 21.9 / 22.8 |
| Plain saved transcript | 10,000 | 195.9 / 218.7 | 158.6 / 184.9 |
| Arabic with Latin fixture markers | 1,000 | 51.5 / 52.5 | 44.7 / 46.8 |
| Indic with Latin fixture markers | 1,000 | 66.6 / 67.5 | 54.0 / 54.8 |
| CJK with Latin fixture markers | 1,000 | 56.8 / 60.0 | 53.1 / 56.2 |
| Emoji with Latin fixture markers | 1,000 | 40.5 / 42.2 | 29.3 / 34.7 |

The 10,000-row median falls 19.0%; p95 falls 15.5%. Its forced-layout component
falls from 135.0 / 155.7 to 99.5 / 111.5 ms p50/p95. Small fixtures remain
mixed: the 100-row tail is worse and the 1,000-row median is unchanged. The
mixed-script fixtures contain Latin markers rendered by Neon; their results
do not imply faster shaping inside the unchanged fallback fonts.

Every accepted fixture retains identical complete text hashes, row HTML hashes,
row counts and transcript heights. The 10,000-row case retains 1,063,780 row
characters. A first selected-meeting open after reload, with fonts already
ready, is recorded separately: 304.2 → 259.6 ms, one observation per variant.
The new face is loaded at the first measured layout, and neither variant makes
an additional Neon request during the click. Waiting for `document.fonts.ready`
finishes at 306.9 → 263.6 ms including the subsequent layout check; no font-load
deferral explains this observation. This is not a cold network or WASM startup
measurement. The loading status accounts for a transient 26 px height in both
variants; final heights match after it settles.

Whole-transcript selection remains complete: all 1,065,530 characters including
speaker labels, with selection plus forced range geometry 13.6 → 12.2 ms
(one observation each). No containment or partial-history selection is used.

### Live delivery, persistence and fallback checks

The disposable fixture sends actual named `event: meetings` SSE frames through
the application's EventSource decoder and transcript reconciliation. Both
variants receive seven partial updates, a final segment and a newly appended
row. All received text is complete, partial styling and confidence badges
transition correctly, and an out-of-order partial replay does not replace the
final segment. The final variant's partial delivery-to-DOM-plus-layout samples
are 0.2–0.7 ms; completion is 0.3 ms and append is 0.8 ms. These nine samples
are behavioral diagnostics, not tail-latency or network benchmarks.

An earlier selected row retains its DOM node, text node, selection and exact
screen/scroll position through partial updates, finalization and append. The
speaker rename uses the actual editor, with the server response held until a
reader selection is established after Enter. Releasing the authoritative
response retains that selection and the selected row's DOM/text identities
with zero-pixel screen/scroll movement. A separate read-only IndexedDB transaction verifies
the renamed speaker and both completed segments after the application's save;
navigating away and reopening retains all 1,001 rows, final text and 25 updated
speaker chips. This checks acknowledged saved state separately from the
opening/streaming clock.

Ten paired font samples preserve identical width and height with the original
and final faces: Latin ligatures/operators, Arabic, Indic, CJK, family emoji,
skin-tone/ZWJ emoji, flags, combining Latin and Hebrew/mixed text. The transcript
element's feature properties remain `normal`; only its primary face has the
two disabled optional features. A 12-row edge fixture retains every original
character, including literal HTML, newlines, tabs and a 2,000-character word.
Its apparent image/script text creates no elements, executes no code and
requests no injected resource. Complete selection passes. This is sampled
fallback compatibility in this browser, not coverage of all scripts/platforms.

At 390 × 844, the Arabic, Indic, CJK and emoji fixtures each retain all 1,000
rows, expose the last row, support complete transcript selection, and have no
document or transcript horizontal overflow. The responsive navigation is used
to reach each fixture. Browser JavaScript errors and external resource requests
remain zero. The disposable server and owned browser tabs were closed after
validation.

The production Trunk build and quick gate pass: **2,348 Rust tests, zero failures,
17 ignored, plus 36 site tests**, native/WASM Clippy with warnings denied,
formatting, site type checking and benchmark self-checks. Test totals include
concurrent workspace additions. No new Rust tests were added for this CSS-only
change. Whitespace checks pass. Logs are
`target/performance-transcript-face-build.log`,
`target/performance-transcript-gate.log` and `target/performance-gate-pass10/`.

Reproduction inputs are `target/performance-transcript-fixture.py`,
`performance-transcript-events-probe.js`, `performance-transcript-live-checks.js`
and `performance-transcript-face-suite.js`, together with the retained base HTTP
fixture and mount probe. The synthetic server runs on port 49466 to match the
production `BRUHS_API_URL`. Select each distribution through
`performance-browser-current.txt`, reload, install the suite, launch the named
`__launchTranscriptFaceSuite(label)` job and collect it after completion. Live
fixture mutations are in memory; use a fresh server/fixture identity before
replaying partial-to-final sequences. Raw measurements and accepted comparisons
are `performance-transcript-face-{before,after,comparison}.json`; live, rename,
edge, shaping and mobile results have matching `performance-transcript-*.json`
names. Rejected CSS/font pilots remain separate.

Build space was restored by removing two obsolete web debug incremental caches
while holding their target's nonblocking Cargo lock. The current incremental
cache, native caches, sources, final distributions and all measurement evidence
were preserved. Approximately 6 GB was recovered for the completed build.

## Native streaming parsing and selection geometry pass

The eleventh pass removes repeated Markdown work and a quadratic caret lookup
from native reply updates. The event reducer itself is small; the cost is in
the foreground work triggered by a repaint. An entity-creation observer confirms
that the Markdown view already survives streamed tokens and completion, including
a 1,000-line code block. Replacing that view was not the bottleneck.

`TextViewState` now reuses earlier parsed blocks when a full-value update appends
to a paragraph separated by a blank line. Its complete source stays available,
and first layout remains synchronous. Reference definitions, footnotes, custom
extensions and HTML retain full parsing. Container transitions outside the
paragraph guard also use the complete parse. A naive tail
prototype failed a list/lazy-continuation case; the shipped guard also checks
the preceding blank line and the new tail's first block. Reference context is
retained with the parsed document rather than discarded after parsing.

The block splitter also checks potential fence info strings before building a
second Markdown AST. An ordinary mention of “mermaid” no longer triggers that
parse. Case-insensitive tags, escaped/encoded info strings, longer tilde fences,
and CR/LF line endings still reach the Markdown parser. The closing-fence check
reads from the end and recognizes CR-only fences during streaming.

Selectable native text previously searched all preceding logical lines for
each character's position on every paint, including unchanged code. The new
geometry walker indexes shaped lines once, skips offscreen logical lines, and
calculates caret positions for the visible wrapped range and its boundaries.
The resulting clipped bounds and selection centers match the complete scan.
Widths use global positions to preserve the original floating-point rounding.
Alternate/truncated source mappings retain the previous calculation. This
changes geometry work, not the stored text or selection extent.

Background `push_str` delivery retains delta-sized messages. It carries a parsed
starting point once after a synchronous replacement; the worker keeps complete
source separately from successful parse results. This preserves initial and
replaced content, coalesced chunks and recovery text without copying the whole
document into every queued message. The intermediate full-snapshot prototype
was rejected. Revision checks still reject older results after replacement.

### Paired optimized measurements

The accepted comparison replays the retained before and final optimized test
binaries sequentially after compilation and gate work finish. Each fixture has
three warm-up text appends followed by **21 measured appends**, then one completion
event. Each append is a short UTF-8 suffix on an already mounted reply. The clock
includes direct event reduction, notification and GPUI `run_until_parked` rendering
work at 1280 × 800. Initial mounting is outside that clock. The diagram fixture
holds SVG workers in both variants, so it measures the foreground pending-source
view, not diagram-ready latency. Percentiles use nearest rank.

These are GPUI test-platform computation/layout/paint-callback diagnostics.
They exclude provider/channel delivery, durable saves, physical OS input, GPU
presentation and real-frame acceptance. They do not establish INP or that every
action is already optimal. Both binaries use the same workspace dependency
configuration; the source changes during the comparison are the parsing/geometry
implementation and its tests. Other source hashes and the immutable binaries
are retained for inspection.

| Existing reply during a text append | Initial UTF-8 bytes | Before p50 / p95 (ms) | Final p50 / p95 (ms) |
| --- | ---: | ---: | ---: |
| 100 prose paragraphs | 10,590 | 2.978 / 3.602 | 1.413 / 2.008 |
| 1,000 prose paragraphs | 106,890 | 31.994 / 33.000 | 14.461 / 17.723 |
| Closed 1,000-line Rust block plus a prose tail | 55,797 | 78.794 / 79.658 | 0.988 / 1.576 |
| 1,000 paragraphs mentioning “mermaid” | 106,918 | 44.469 / 47.279 | 14.715 / 16.328 |
| Closed Mermaid fence plus 1,000 paragraphs | 106,924 | 45.153 / 46.002 | 27.893 / 28.904 |

The isolated application block-splitter stage for the ordinary “mermaid” mention
falls from **13.540 / 14.168 to 0.471 / 0.487 ms** p50/p95. This stage excludes
the text widget's own Markdown parse and layout. The actual diagram's splitter
still costs **13.720 / 15.381 ms** in the final replay; removing its repeated
full AST construction remains open. Small changes in unchanged splitter paths
are not attributed as improvements.

Completion with unchanged closed-code content costs **77.604 → 0.997 ms** in one
observation per variant, consistent with removing geometry work that previously
ran on every paint. Other completion observations are 2.068 → 1.537 ms for 100
paragraphs, 16.644 → 13.490 ms for 1,000 paragraphs, 29.800 → 13.701 ms for the
ordinary Mermaid mention, and 28.901 → 28.194 ms for the diagram fixture. These
single observations are not completion percentiles; the diagram's completion
cost is materially unchanged.

### Behavioral receipts and reproducibility

Five permanent native integration contracts verify:

- **11,118 streamed UTF-8 prefixes** match fresh complete parses, including
  source, blocks, spans, formatting marks and reference context. Fixtures cover
  definitions added later, setext headings, tables, lists/lazy continuation,
  HTML, code fences, Unicode and mixed CR/LF line endings, plus deterministic
  generated combinations.
- **160 clipped shaped-text geometries** match the complete per-character scan,
  including wrapped text, newlines, combining text, CJK, emoji, Arabic/Hebrew,
  horizontal clipping, vertical offsets and differing line heights. These use
  the GPUI test text system; they are not platform/font coverage claims.
- A mouse drag selects visible code in a long block and retains exactly that
  selected text through a streamed prose-tail update and completion.
- One Markdown view survives updates and completion of a 1,000-line code reply.
- Background appends retain initial/replaced Markdown, references, all chunks
  in a 100-chunk burst, newer synchronous replacements and earlier closed HTML
  blocks. The separate fence probe checks both streaming and settled parsing.

Every timing fixture also checks the complete final message against its expected
source after all appended chunks and completion. The parser reuses complete
blocks; selection geometry does not truncate the document or cap history.

The final quick gate passes **2,354 Rust tests, zero failures, 18 ignored, plus
36 site tests**, formatting, native/WASM Clippy with warnings denied, site type
checking and benchmark self-checks. The optimized build passes, and its five
streaming contracts pass separately. Changed vendored Rust files also pass an
explicit rustfmt check because that package is excluded from workspace formatting.
Whitespace checks pass. The new ignored timing fixture accounts for the additional
ignored test. Test totals include prior concurrent workspace additions.

Reproduce the diagnostic with:

```sh
cargo test -p bruhs-desktop --bin bruhs-desktop --release --locked native_streaming_action_cost -- --ignored --nocapture
cargo test -p bruhs-desktop --bin bruhs-desktop --locked streaming_costs -- --nocapture
scripts/gate.sh --quick
```

The fixture is
`bruhs-desktop/src/views/chat/behavior_tests/streaming_costs.rs`. Accepted raw
logs/results are `target/performance-native-stream-{before,after}-paired.{log,json}`;
the comparison and binary manifest are
`target/performance-native-stream-comparison.json`. Before/final binary SHA-256:
`7d9d49b5ed18244f890c9e87724a3b24d53808058077759f606a07d7e143c130` /
`b2ba11e56029f48be953fd470dcb853bc2569cee2595d431149f92508522a062`.
The initial baseline and rejected snapshot prototype remain separate. Final
logs are `performance-native-stream-final-build.log`,
`performance-native-stream-release-contracts.log`,
`performance-native-stream-gate-final.log` and `performance-gate-pass11/`.

### Action coverage discovery

A read-only source catalogue now records **43 palette entries on each UI,
19 shared views, eight slash commands, 94 server HTTP method/path entries,
32 MCP tool names and ten workflow operation variants**. It also finds 451 web
and 585 native event-binding references. Source locations and SHA-256 hashes are
retained in `target/performance-action-inventory.json`; regenerate with
`python3 target/performance-action-inventory.py`.

This catalogue guides the remaining audit. It deliberately does not mark an
entry measured from the presence of a unit test, a nearby benchmark or a green
gate. The binding scan is a text search rather than a Rust AST; nested commands,
parameter branches, shortcuts, subscriptions, error/recovery/lifecycle states,
direct capture/provider interfaces, and bridge/standalone MCP modes still need
semantic expansion and explicit evidence mapping. Counts overlap across
surfaces and are not a completion percentage. All-actions completion remains
unproven.

## Acknowledged save and model-checksum pass

The twelfth pass, continued on November 10, measures the native save worker
through real temporary-file completion. Earlier snapshot and serialization
fixtures did not include this filesystem work. The measured implementation
retains compact JSON from pass six, full SHA-256 conflict detection, the
nonblocking process lock, private temporary files, the previous-state backup,
atomic replacement and backup/file/directory synchronization.

### Verified fixes

The locked `sha2` 0.10.9 dependency selected its software SHA-256 implementation
on aarch64 because its `asm` feature was disabled. The crate's local
`sha256.rs` and `sha256/aarch64.rs` show that `asm` enables a runtime-detected
SHA2 backend with the software fallback retained. SHA2 instructions are available
on this machine. Enabling that feature specifically for macOS aarch64 in
`bruhs-core` and standalone `bruhs-capture` removes repeated software hashing
from save fingerprints and cached-model verification. The checksum algorithm
and the complete file checks remain the same. The lockfile adds `sha2-asm`
0.6.4; no dependency version was upgraded. The Intel Mac feature graph was
checked separately and does not gain the feature. Other targets were not
compiled as part of this pass.

An obsolete save request now checks its generation before serialization.
The existing check under the writer mutex still runs after encoding, so a
generation that changes during serialization cannot install stale bytes.
Discarding an already obsolete request still drops its owned snapshot; that
cost remains visible in the measurements.

### Paired optimized measurements

The new ignored `acknowledged_save_action_cost` fixture uses synthetic people
with twenty observations and a voice vector each, and synthetic finance records
with complete memos. It takes an actual workspace snapshot, separately measures
its encoding for the expected-byte check, then runs the real save function in
a worker thread. Worker time includes its own encoding, both SHA-256
fingerprints, the writer protocol, backup/file/directory sync calls, and snapshot
destruction. It excludes thread creation/join, the production 150 ms debounce,
queue/dispatch delay, UI receipt, physical input and paint. These are warmed
local temporary files, not cold-device or power-loss measurements.

Each fixture performs nine acknowledged writes per binary, discards the first
two for timing, and checks the complete installed bytes after every write. The
backup must equal the exact previously acknowledged state. Seven obsolete-save
samples per fixture must leave the installed bytes unchanged. p95 uses nearest
rank; with seven samples it is the observed maximum.

The accepted before and after binaries ran sequentially with compilation
stopped. Their manifests record start/finish times and approximately 6.4 GB of
free space before each run. An earlier pair accidentally overlapped and was
rejected; the initial after-build run also overlapped compilation and is not
used for the comparison. Obsolete compiler caches from earlier audit passes
were removed before the accepted pair. Filesystem tails remain variable: the
initial baseline worker measurements were lower than those in this later pair.
The full-worker figures are a local diagnostic, not a stable latency threshold
or an attribution of every tail change to hashing.

| Synthetic history | Encoded bytes | Worker before p50 / p95 (ms) | Worker after p50 / p95 (ms) |
| --- | ---: | ---: | ---: |
| 1,000 people | 12,721,785 | 237.308 / 269.079 | 157.991 / 233.935 |
| 5,000 people | 63,685,785 | 945.617 / 2,372.042 | 767.372 / 874.675 |
| 10,000 finance records | 4,402,989 | 102.084 / 106.357 | 74.913 / 81.543 |
| 100,000 finance records | 44,123,015 | 723.680 / 876.010 | 529.766 / 868.352 |
| 2,500 people + 50,000 finance records | 53,885,526 | 743.919 / 932.994 | 478.813 / 673.349 |

Encoded lengths match across this pair. Each run checks its own expected
complete payload and preceding backup; equal lengths are not a claim that
independently constructed workspace metadata is byte-identical across runs.

| Already obsolete request | Before p50 / p95 (ms) | After p50 / p95 (ms) |
| --- | ---: | ---: |
| 1,000 people | 9.081 / 9.723 | 0.656 / 0.861 |
| 5,000 people | 44.465 / 46.225 | 3.156 / 3.599 |
| 10,000 finance records | 3.676 / 3.753 | 0.483 / 0.494 |
| 100,000 finance records | 35.019 / 35.596 | 5.322 / 5.630 |
| 2,500 people + 50,000 finance records | 41.800 / 44.237 | 4.495 / 4.746 |

Foreground snapshot ownership has not changed. For 5,000 people, snapshot
construction is still **6.953 / 9.641 ms** p50/p95 after the change; for 100,000
finance records it is **7.926 / 8.196 ms**. The retained per-record shared
ownership prototype has thirty-one clone/drop and one-record copy-on-write
samples, but is not shipped. That cross-model ownership change needs its own
behavioral and foreground measurements. Encoding also remains roughly 43 ms
at the median for the largest people fixture.

The separate primitive diagnostic uses the exact retained software and final
accelerated native release `sha2` libraries, the same optimized diagnostic
source, and seven digest samples. Each result matches Python `hashlib` as well
as the other backend:

| SHA-256 input | Software p50 / p95 (ms) | Accelerated p50 / p95 (ms) |
| --- | ---: | ---: |
| 4 MiB | 11.370 / 11.615 | 1.874 / 1.897 |
| 16 MiB | 45.148 / 45.607 | 7.330 / 7.548 |
| 64 MiB | 180.693 / 187.588 | 29.520 / 30.223 |

This isolates an approximately sixfold hashing improvement on this machine;
the complete save also contains serialization and variable filesystem work.

The new ignored `model_checksum_action_cost` fixture runs the actual async
cached-model verifier on a synthetic 64 MiB file. After two warm calls, seven
samples give **33.308 / 35.162 ms** p50/p95, including blocking-worker dispatch,
length validation, reading, and hashing. Changing only the final byte while
retaining file length is rejected as corrupt. This is an after-only action
measurement; there is no before/after claim for the complete verifier, model
loading, microphone capture, transcription or inference. The capture diagnostic
ran through Cargo so its native dynamic-library search paths were supplied.
A preliminary invocation targeted the library's zero matching tests and is
retained separately as rejected evidence.

### Verification and evidence

The new ordinary disk-version test checks independently known SHA-256 values
for empty input, `abc`, and a 16,401-byte payload crossing the streamed-reader
buffer. In-memory and file fingerprints must both match. All eleven
disk-version/conflict contracts pass in debug and the retained optimized
binary, including separate-process stale writers,
deletion/symlink changes, competing writers, interrupted lock ownership, and
directory-sync failure/retry.

The final quick gate exits successfully: **2,356 Rust tests passed, 0 failed,
20 ignored; all 36 site tests passed**, with formatting, native all-targets
Clippy, WASM Clippy, site types and benchmark self-checks passing. The new
ignored save and model diagnostics were also executed explicitly in optimized
builds. The final source corrects an equivalent benchmark-only integer
conversion from `TryFrom` to `From` after the native optimized artifact was
built; the measured production implementation is unchanged. The gate compiles
and validates the corrected fixture. `git diff --check` passes.

Evidence lives in ignored `target/` artifacts:

- `performance-save-{before,after}-paired.{log,json}`,
  `performance-save-paired-run-manifest.json`, and
  `performance-save-comparison.{json,html}` contain raw samples, summaries, binary
  identities, timing scope and source fingerprints. The before binary SHA-256
  is `6e690c1d2e659cbe24d59471220c89a11406f520461b32951a7f9d267f3aada2`;
  the after binary is
  `dc3c2aaaed7b5e7ef7188f4366220cbe9720f6df8e8a8e7dfef9aa0b6b5463ff`.
- `performance-save-before-source/`, `performance-save-baseline.log`,
  `performance-save-after-build.log`, and
  `performance-save-{before,after}-overlap-rejected.log` preserve baseline
  source and separate nonaccepted timing runs.
- `performance-sha-{before,after}-paired.log`, `performance-sha-paired.json`,
  and `performance-sha-cost.rs` record the primitive comparison;
  `performance-sha-{intel,capture}-features.log` record target feature graphs.
  The earlier isolated feature pilot remains separate.
- `performance-capture-hash-{build,after}.log`,
  `performance-save-{contracts,release-contracts}.log`,
  `performance-save-gate-final.log`, and
  `performance-gate-pass12-final/` contain optimized capture, conflict and gate
  evidence. The wrong-target capture attempt has separate rejected files.

The source action inventory has been regenerated against 349 source hashes;
its action/binding counts are unchanged. Entries still require explicit
semantic evidence mapping. Warm save completion and cached verification extend
the coverage; they do not establish cold storage, capture throughput, provider
time-to-first-event, Google sync, physical UI latency or all-actions completion.

## Shared foreground snapshots and native finance reads

The thirteenth pass removes the measured foreground copies of complete person
histories and finance transactions. Native people caches and `PeopleSnapshot`
now retain `Arc<PersonProfile>` records. `FinanceLedger` retains
`Arc<FinanceTransaction>` records on both surfaces, with Serde's `rc` support
enabled explicitly in the shared model crate. Snapshots clone record handles;
edits use copy-on-write for the affected record, and refresh/create/merge
completions replace handles. An earlier snapshot keeps its complete originals.
New contact capture moves its owned profile into the cache without an extra
profile clone.

The JSON record arrays keep their existing shape. New contracts compare them
with serialized owned records, round-trip existing JSON, preserve extreme money
values, and retain complete observations, voice vectors and Unicode text across
edits and remote replacement. Exact merge archives remain owned originals.
People UI, Today follow-ups and merge preflight now consume borrowed/shared
profiles; web finance entry, CSV insertion and workflow writes retain the new
record handles. This does not bound or truncate stored records.

### Foreground and worker measurements

The retained phase-twelve optimized binary is the before executable; its
original record ownership and primary save fixture were inspected before
editing. The after executable uses the same five synthetic histories and timed
save stages. Earlier flat-record prototypes were removed from the diagnostic
after the production ownership change. Each fixture still checks every complete
acknowledged payload and its exact preceding backup, using nine writes with
two warm writes and seven measured samples. Encoded lengths match across these
runs. Expected complete bytes are checked within each run, rather than assumed
identical across independently constructed workspaces.

The binaries ran sequentially with Cargo profile locks held and no observed
build processes. A reverse-order replay investigates a worse worker/encoding
tail in the first comparison. Both orders remain recorded. p95 is nearest-rank
and therefore the observed maximum of seven samples.

| Foreground snapshot | Before p50 / p95 (ms) | After p50 / p95 (ms) |
| --- | ---: | ---: |
| 1,000 people | 1.245 / 1.420 | 0.024 / 0.044 |
| 5,000 people | 6.394 / 9.808 | 0.053 / 0.058 |
| 10,000 finance records | 0.820 / 0.995 | 0.061 / 0.065 |
| 100,000 finance records | 7.681 / 8.189 | 0.520 / 0.576 |
| 2,500 people + 50,000 finance records | 7.827 / 8.874 | 0.290 / 0.424 |

| Foreground snapshot, reverse run order | Before p50 / p95 (ms) | After p50 / p95 (ms) |
| --- | ---: | ---: |
| 1,000 people | 1.244 / 1.332 | 0.022 / 0.076 |
| 5,000 people | 6.504 / 9.388 | 0.048 / 0.051 |
| 10,000 finance records | 0.857 / 0.888 | 0.054 / 0.086 |
| 100,000 finance records | 8.001 / 8.271 | 0.567 / 0.609 |
| 2,500 people + 50,000 finance records | 7.366 / 7.643 | 0.300 / 0.332 |

Already obsolete requests also avoid destroying deep copies of all records:

| Obsolete request, first run order | Before p50 / p95 (ms) | After p50 / p95 (ms) |
| --- | ---: | ---: |
| 1,000 people | 0.551 / 0.592 | 0.003 / 0.006 |
| 5,000 people | 3.163 / 3.427 | 0.008 / 0.013 |
| 10,000 finance records | 0.508 / 0.818 | 0.018 / 0.031 |
| 100,000 finance records | 5.244 / 5.915 | 0.273 / 0.373 |
| 2,500 people + 50,000 finance records | 4.398 / 4.499 | 0.110 / 0.139 |

Snapshot construction runs on the GPUI test thread. Worker timings still
include encoding, guarded writes, synchronization and snapshot destruction;
they exclude debounce, queue/dispatch, UI receipt and physical input/paint.
These fixtures establish a reduction in foreground snapshot work, not complete
action latency or frame rate. Record construction is outside their timers;
the extra shared allocations, memory use, CSV/import throughput and dense People
page rendering need their own measurements.

The first 1,000-person worker result rose from **47.946 / 49.739 ms** to
**85.899 / 166.197 ms** p50/p95. In reverse order it was
**47.895 / 48.571 ms** before and **49.349 / 52.432 ms** after. Its independent
encoding result also changed from **8.346 / 8.602** to **9.537 / 11.656 ms** in
the first order, but **8.341 / 8.651** to **8.290 / 8.570 ms** in reverse order.
The first regression did not reproduce at that magnitude. Full-worker results
remain variable and do not support a universal save-worker or tail-latency
improvement claim. All stage samples, including the worse run, are retained.

### Native finance read-handler boundary

The existing optimized `life_read_handler_large_ledger_measurement` fixture also
ran from both binaries. It uses 100,001 records, five warm requests and fifty
measured requests per query, and checks returned record counts. Timing spans
native handler entry through the JSON reply, including grant/privacy checks,
filtering and paging. It excludes socket/stdio transport, hydration, file I/O
and paint. The fixture logs rounded summaries rather than raw samples; its
p50/p95/p99 are sorted indexes 24/47/49.

| Read query | Before p50 / p95 (ms) | After p50 / p95 (ms) |
| --- | ---: | ---: |
| First page | 0.070 / 0.127 | 0.069 / 0.071 |
| Deep page, offset 99,950 | 0.205 / 0.215 | 0.201 / 0.228 |
| Match at final record | 8.769 / 9.106 | 8.957 / 9.298 |
| No matching record | 12.579 / 13.152 | 12.654 / 13.038 |

This is additional boundary evidence, not a read-handler speedup claim.
Full keyword scans remain around 9–13 ms. Their repeated field normalization is
a concrete next profiling target; transport and cold initialization remain
separate measurements.

### Checks and reproducible action discovery

The successful quick gate exits zero: **2,365 Rust tests passed, 0 failed,
20 ignored; all 36 site tests passed**. Formatting, native all-targets Clippy,
WASM Clippy, site types and benchmark self-checks pass. The new native
snapshot-isolation contract also passes in the optimized executable. The
production WASM/asset build succeeds using installed Trunk 0.21.14 with
`NO_COLOR=true` to satisfy its Boolean environment parser. No provider or real
user data was used by these diagnostics.

An earlier debug link exhausted disk space. Inactive native incremental caches
were cleared while holding the target's exclusive Cargo lock; sources,
compiled measurement binaries and evidence were retained. The accepted builds
and timings follow that cleanup. Early failed attempts remain separate from
the successful gate/build logs.

The catalogue generator now lives in
`scripts/performance-action-inventory.py`; run it from any working directory
with Python 3. It writes only `target/performance-action-inventory.json`, creates
that output directory when needed, links all 43 web and 43 native palette
entries to their dispatch source spans, and fails on a missing or uncatalogued
palette dispatch. Source hashes and span hashes support later evidence review.
It currently discovers 94 HTTP method/path pairs, 86 palette entries, 19 views,
9 slash commands, 32 MCP tools, 10 workflow variants and 1,036 binding references
against 349 source hashes. The ninth slash command comes from other work in
the shared tree. Dispatch spans are text searches, not a Rust AST or transitive
call graph; every entry still requires semantic evidence mapping. The counts
are not a completion percentage.

Evidence remains in ignored `target/` artifacts:

- `performance-snapshot-{before,after}-save-paired.{log,json}`,
  `performance-snapshot-{before,after}-reverse.{log,json}`,
  `performance-snapshot-{paired,reverse}-manifest.json`, and
  `performance-snapshot-comparison.{json,html}` retain both orders and summaries.
- `performance-snapshot-{before,after}-finance-read-paired.log` retains all
  native finance-handler summaries.
- `performance-snapshot-before-source/` and
  `performance-snapshot-before-manifest.json` capture 529 starting source files.
  The before binary SHA-256 is
  `dc3c2aaaed7b5e7ef7188f4366220cbe9720f6df8e8a8e7dfef9aa0b6b5463ff`;
  the after binary is
  `6d69ee9af1ef58a868f07c0c26f64d9851266b70490a93ccbfb26c91be6c1387`.
  Independent thread-role work is also present in the shared tree; record
  ownership, fixture inputs and timing scope are explicit rather than treated
  as a clean-commit comparison.
- `performance-snapshot-after-build2.log`,
  `performance-snapshot-web-build2.log`,
  `performance-snapshot-release-isolation.log`,
  `performance-snapshot-gate-verified.log`, and
  `performance-gate-pass13-verified/` contain successful optimized, production
  WASM and gate results. `performance-snapshot-cache-cleanup.json` records the
  locked cache cleanup.

## Native read-query matching and Unix-socket boundary

The fourteenth pass removes repeated lowercase allocations from native bridge
search. The inspected finance handler spent roughly 9–13 ms scanning 100,001
records, allocating normalized copies of each searched field. A request now
normalizes its query once into `LowercaseQuery`. Ordinary ASCII fields use
`memchr` candidate lookup and ASCII case comparison over borrowed bytes.
Overlapping failed candidate windows resume the built-in normalized search
after the start just ruled out, avoiding repeated speculative prefix scans.
There is no input truncation or new operational limit.

Non-ASCII fields retain the original whole-string `to_lowercase().contains`
behavior, including contextual Greek sigma and expanding mappings. Whitespace,
literal substring matching, normalization policy and the selected search fields
stay the same. The helper is used by native finance/health sections,
people/chat/meeting/note discovery, and generic task/calendar/capture/knowledge/
inbox reads. Generic reads still serialize records for their existing full-JSON
search semantics; that cost remains. `memchr` 2.8.0 was already locked and
transitive; the desktop crate now declares it directly. No package version was
upgraded.

### Optimized native handler comparison

The retained phase-thirteen after binary is the before executable. The existing
large-ledger fixture runs the same 100,001 synthetic ASCII records and four
queries. Its dataset construction was extracted into a shared test helper;
setup remains outside the timer. Each query has five warm calls and fifty
measured calls. The fixture logs rounded summaries at sorted indexes 24/47/49
for p50/p95/p99. Both orders ran sequentially with Cargo profile locks held and
no observed build processes.

Timing spans native handler entry through the JSON reply, including
grant/privacy checks, filtering and paging. It excludes socket/stdio transport,
hydration, file I/O and physical input/paint. Counts are checked for every reply.

| Query, before then after | Before p50 / p95 (ms) | After p50 / p95 (ms) |
| --- | ---: | ---: |
| First page, empty query | 0.068 / 0.069 | 0.069 / 0.078 |
| Deep page, offset 99,950 | 0.203 / 0.231 | 0.158 / 0.166 |
| Match at final record | 9.049 / 9.347 | 1.364 / 1.489 |
| No matching record | 12.565 / 13.061 | 2.013 / 2.158 |

| Query, reverse run order | Before p50 / p95 (ms) | After p50 / p95 (ms) |
| --- | ---: | ---: |
| First page, empty query | 0.070 / 0.087 | 0.069 / 0.081 |
| Deep page, offset 99,950 | 0.204 / 0.218 | 0.158 / 0.200 |
| Match at final record | 8.832 / 9.071 | 1.362 / 1.526 |
| No matching record | 12.583 / 13.189 | 2.006 / 2.275 |

These keyword scans improve by approximately sixfold at this boundary. The
first page is effectively unchanged. Large Unicode-ledger latency and other
read families need separate action measurements; the shared helper alone does
not establish their speedups.

### Real Unix-socket round trip

The new ignored `life_read_native_ipc_cost` fixture also uses 100,001 synthetic
records. It enables an isolated real bridge session, then uses the existing
test client helper for five warm requests and fifty measured requests per
query. Each timed call loads the private connection file, constructs a client,
connects over the Unix socket, crosses foreground dispatch, receives/decodes
the reply, and returns through a test-executor oneshot. This includes fixture
scheduling overhead. Connection enablement, initial dataset hydration, MCP
stdio, provider calls and physical paint are outside the timer.

| After-only Unix-socket request | p50 (ms) | p95 (ms) | p99 (ms) |
| --- | ---: | ---: | ---: |
| First page | 0.176 | 0.321 | 0.404 |
| Deep page | 0.279 | 0.358 | 0.425 |
| Match at final record | 1.478 | 1.605 | 1.934 |
| No matching record | 2.111 | 2.545 | 2.781 |

These use nearest-rank percentiles from retained raw samples. The fixture checks
record counts, first IDs and amount redaction; a later request after Read
revocation is rejected. Dropping the session cleans up its own socket and
connection file. This is after-only transport evidence, not a transport
before/after or complete MCP tool latency claim.

### Correctness, candidate selection and checks

The matcher compares against the previous implementation for 16,770 ASCII
substring/case variants and 20,000 generated mixed-text query cases. Contracts
also cover empty fields, NUL/control bytes, significant whitespace, dotted I,
Kelvin sign, non-equivalent full-case-fold examples, composed/decomposed accents,
emoji, contextual sigma, long repeated prefixes and late overlapping matches.
Native integration checks preserve metadata-only search and redaction; generic
JSON search retains its original escaping/casing semantics.

A reusable normalization buffer and a cumulative-comparison-budget byte
prototype were measured before selection. The buffer left substantial repeated
work; the budget prototype added too much work to long-prefix cases. The
overlap fallback retains the ordinary ASCII gain while handing those cases
back to the standard search promptly. Prototype logs remain separate from the
accepted full-handler measurements. Unicode fallback retains its allocation
cost; no cache or cross-request state was introduced.

All four standalone matcher contracts pass. The targeted native bridge run
passes **107 tests, 0 failed**, and the real-socket fixture also passes in debug.
The successful quick gate reports **2,371 Rust tests passed, 0 failed,
21 ignored; all 36 site tests passed**, with formatting, native all-targets
Clippy, WASM Clippy, site types and benchmark self-checks passing. Optimized
native tests build successfully, and the new ignored IPC fixture was explicitly
executed in that build. `git diff --check` passes. All data and connections in
these diagnostics are synthetic and isolated.

The source catalogue now has a reviewed partial-evidence ledger at
`docs/performance-action-evidence.json`. Its first entry links the native
finance transaction variant to the handler and socket measurements, with
unmeasured sections, backends and boundaries listed explicitly. The generator
checks listed source hashes, flags changed sources and reports missing local
artifacts. An isolated copy with a changed matcher source verifies that stale
evidence is flagged. These checks pin listed inputs; they are not a complete
transitive dependency or runtime acceptance check. The catalogue still has
94 HTTP method/path pairs, 86 palette entries, 19 views, 9 slash commands,
32 MCP tools, 10 workflow variants and 1,036 binding references, now against
350 source hashes. One partially measured variant is not whole-action or
all-actions completion.

Evidence remains in ignored `target/` artifacts:

- `performance-query-{before,after}-{forward,reverse}.log`,
  `performance-query-ipc-after.log`, `performance-query-runs.json`, and
  `performance-query-comparison.{json,html}` contain accepted summaries, raw
  IPC samples, run order, timing scope and source fingerprints.
- `performance-query-before-source/` and
  `performance-query-before-manifest.json` retain 14 starting source files and
  the inspected baseline identity. Before binary SHA-256:
  `6d69ee9af1ef58a868f07c0c26f64d9851266b70490a93ccbfb26c91be6c1387`;
  after:
  `61d89064d3383604bcb91dc0748e315d1d5f348a3add1e01404b4074c3dbe366`.
- `performance-query-prototype{,2}.log` and
  `performance-query-prototype.rs` retain candidate diagnostics;
  `performance-query-properties.log`, `performance-query-contracts.log`, and
  `performance-query-ipc-debug.log` record correctness checks.
- `performance-query-after-build.log`, `performance-query-gate.log`, and
  `performance-gate-pass14/` contain optimized-build and gate evidence.

## MCP stdio discovery and full native read boundary

This loop measured the actual optimized `bruhs-mcp` executable over newline JSON-RPC pipes, in standalone mode and native bridge mode. Bridge startup already skips `AppState` construction; the standalone constructor creates in-memory state without starting background sync. No provider, real user record, or external service was used. The baseline is HEAD `bcfce5d387ac4cfdfeae4945615cf37780e158e7` plus the existing dirty tree and all earlier performance changes.

`tools/list` was cloning every tool description/schema, converting the catalog into a JSON value, and cloning that value again through the generic result encoder. The registry now lends its immutable `ToolInfo` entries to a typed catalog wrapper, which goes through the result encoder once. `tools/call` also consumes its owned request ID and parameter object rather than cloning them before deserialization. The registry remains the common source for listing and dispatch; mode availability, field schemas, permissions, response framing, errors and flush behavior are preserved.

A single-thread Tokio runtime prototype was rejected and reverted: its exploratory fresh-process initialization measurements did not show a consistent gain. The final build retains the original runtime. The large invalid-query control also shows no consistent tail improvement from moving parameters, so no separate request-parsing speedup is claimed.

### Tool discovery through real pipes

Each run uses 10 warmups and 1,000 measured complete-frame round trips per case. Timers start before the parent writes/drains the request and end when it receives the complete stdout line. Request encoding and parent response JSON validation are outside these warm timers. The synthetic bridge responder uses a private directory, token file, authenticated Unix socket and the real length-prefixed protocol. It does **not** run the native query handler; the separate native fixture below does. Both run orders ran with all Cargo/rustc builds idle and the four profile locks held.

| Discovery mode / run order | Before p50 / p95 / p99 (ms) | After p50 / p95 / p99 (ms) |
| --- | ---: | ---: |
| Standalone, before → after | 0.158 / 0.211 / 0.278 | 0.117 / 0.155 / 0.185 |
| Bridge, synthetic responder, before → after | 0.245 / 0.314 / 0.376 | 0.150 / 0.201 / 0.247 |
| Bridge, actual native app fixture, before → after | 0.246 / 0.301 / 0.348 | 0.150 / 0.203 / 0.249 |
| Standalone, after → before | 0.158 / 0.204 / 0.261 | 0.115 / 0.151 / 0.180 |
| Bridge, synthetic responder, after → before | 0.252 / 0.322 / 0.368 | 0.151 / 0.197 / 0.258 |
| Bridge, actual native app fixture, after → before | 0.247 / 0.330 / 0.373 | 0.148 / 0.202 / 0.244 |

The synthetic/standalone comparison reduces discovery median latency by about 26–27% in standalone mode and 39–40% in bridge mode in the two run orders. The actual native fixture independently reproduces the bridge discovery improvement. Discovery response sizes stay exactly 10,473 bytes for standalone and 24,672 bytes for bridge mode. Every sampled warm response has the same canonical JSON fingerprint and byte count before and after, including every catalog schema and error result.

Fresh-process startup is a separate measurement: 100 launches per mode/executable after two discarded launches, from parent subprocess creation through a validated `initialize` result. OS caches are warm; parent subprocess and JSON overhead are included. Final initialization medians are about 4.4–4.6 ms and tails vary by run/order. These results do not establish a startup speedup or cold-start bound.

The real standalone process also measures `bruhs_agent_audit` with default permissions and empty history, `bruhs_read_chat` with no threads, `bruhs_list_threads` with no threads, and `bruhs_list_tasks` for a valid ID with no registered tasks. The forward after medians are approximately 0.088, 0.081, 0.082 and 0.082 ms. These are empty-state evidence, not populated-history scalability claims. Ping, unknown-tool rejection and an oversized 60,000-byte query inside an otherwise valid sub-64-KiB request are controls. The query is rejected by the existing 512-byte argument limit as a tool error, without a native read.

### Complete stdio → Unix socket → native foreground reader

The new ignored GPUI fixture constructs the same 100,001-record ASCII ledger, enables Finance Read plus the Life agent-context opt-in, clicks the actual connection control, then launches the Python stdio client and real MCP child. This crosses the real native foreground handler and returns the complete MCP stdout frame. Both MCP versions use the **current** native query implementation. It measures the MCP wrapper change, not another comparison of the earlier query optimization. Record construction, connection enablement, user input/paint and cold persistence hydration are outside the timer.

Each query has 10 warmups and 200 measured samples per executable/order. All returned IDs, full record counts, next offsets, monetary-field omission and private-note omission are checked. Canonical response fingerprints and byte counts match before and after.

| Finance query / run order | Before MCP p50 / p95 / p99 (ms) | After MCP p50 / p95 / p99 (ms) |
| --- | ---: | ---: |
| First page, before → after | 0.281 / 0.389 / 0.469 | 0.287 / 0.368 / 0.431 |
| Offset 99,950, before → after | 0.369 / 0.449 / 0.502 | 0.376 / 0.463 / 0.557 |
| Final-record match, before → after | 1.635 / 1.790 / 1.948 | 1.536 / 1.711 / 1.950 |
| No match, before → after | 2.310 / 2.549 / 2.755 | 2.220 / 2.415 / 2.598 |
| First page, after → before | 0.282 / 0.373 / 0.441 | 0.285 / 0.378 / 0.423 |
| Offset 99,950, after → before | 0.373 / 0.464 / 0.580 | 0.375 / 0.476 / 0.565 |
| Final-record match, after → before | 1.608 / 1.781 / 2.018 | 1.613 / 1.770 / 2.002 |
| No match, after → before | 2.232 / 2.469 / 2.610 | 2.252 / 2.449 / 2.587 |

The read timings are broadly similar across the MCP versions; no consistent query/read speedup is attributed to this pass. The current complete path has warm medians around 0.29 ms for the first page, 0.38 ms for a deep page, 1.54–1.61 ms for the final match and 2.22–2.25 ms for no match. Do not subtract the previous phase’s IPC numbers to infer a transport overhead: its helper creates a fresh bridge client and includes a different test-executor/decoding boundary.

### Verification, evidence mapping and reproduction

The quick repository gate passed: formatting, native all-target/all-feature Clippy with `-D warnings`, WASM Clippy, **2,371 Rust tests passed** (22 manual tests ignored), **36 site tests passed**, site types and benchmark self-checks. The first gate caught an unused import in the new ignored fixture; it was removed before the passing gate and final optimized build. The optimized native test build completed in 8m 07s with the same two existing release-only dead-code warnings in preview/settings helpers.

The existing optimized native IPC fixture was rerun successfully, including grant revocation and session cleanup. The only change to its containing source file is `mod stdio;`; a receipt checks that removing that declaration reproduces the prior fixture hash exactly and that all previously pinned production sources remain unchanged. The historical ledger entry is revalidated with that receipt rather than silently treating the added manual fixture as a query implementation change.

The source inventory now distinguishes four stdio methods (`initialize`, `tools/list`, `tools/call`, `ping`) from the 32 tool names. It still lists 94 HTTP entries, 86 palette entries with 86 dispatch links, 19 views, 9 slash commands, 10 workflow operations and 1,036 binding references, across 352 source hashes. The reviewed ledger adds partial evidence for discovery/initialization, the full native finance transaction path and the four standalone empty-state reads. None becomes whole-action acceptance or a completion percentage.

Preserved optimized binaries:

| Executable | SHA-256 |
| --- | --- |
| Before MCP | `cc2f34c9b212063f3df130ed7795070b6461b06f43fbea60b8d8f09f948fa898` |
| After MCP | `abf04b583af350627454466ca3710b1e081cfbcd0f2cb4ec15d8be4485c30961` |
| Current native fixture | `9c76a151f59b023ed92874166918373628dec35cc4add3a7cb15d7308a296806` |

Raw samples and receipts are under `target/`: `performance-stdio-{before,after}-{forward,reverse}.json`, `performance-stdio-native-{before,after}-{forward,reverse}.json`, `performance-stdio-runs.json`, `performance-stdio-native-runs.json`, `performance-stdio-comparison.json`, `performance-stdio-fixture-revalidation.json`, `performance-stdio-ipc-revalidation.log`, `performance-stdio-gate.log` and `performance-gate-pass15/`. `performance-stdio-before-source/` preserves the MCP baseline source. `performance-stdio-runtime-manifest.json` preserves the rejected runtime experiment. The comparison pins 43 listed source files; it does not claim every transitive dependency or runtime condition is pinned.

Reproduce the current executable’s isolated stdio measurements after builds finish:

```sh
cargo build -p bruhs-mcp --release --locked
python3 scripts/performance-mcp-stdio.py \
  --binary target/release/bruhs-mcp \
  --output target/performance-current-stdio.json \
  --startup-samples 100 --samples 1000
cargo test -p bruhs-desktop --bin bruhs-desktop --release --locked --no-run
```

Then run the optimized native test executable printed by the last command directly, with `BRUHS_PERF_MCP_BIN` set to the absolute optimized MCP path and `BRUHS_PERF_STDIO_OUTPUT` to a new absolute output file, filtering `life_read_native_stdio_cost --ignored --nocapture`. That fixture supplies its own private connection; do not use a real user workspace for these synthetic assertions. The preserved `target/performance-stdio-run.py` and `performance-stdio-native-run.py` drivers reproduce the paired order controls while preventing concurrent Cargo profile writes.

Remaining variants include growing standalone workflow/audit history; other native finance sections and domains; Unicode-heavy data; cold hydration/connection setup; concurrent clients, cancellation, disconnection/error tails; and physical UI input/paint. The subsequent reachability review established that the current standalone catalog cannot populate its thread/task maps. These measured stdio requests close one boundary gap, not the entire action audit.

## Generic native record reads and canonical JSON buffer reuse

This loop measured the five native full-record adapters: Tasks, Calendar, Capture,
Knowledge and Inbox. Unlike the separately projected Finance/Health/People/meeting
reads, these adapters search the entire compact serialized JSON record. Keys, escapes,
nested message text and saved document versions are part of the existing search. The
goal remains active; these are partial action/variant results, not all-actions
acceptance.

The corrected optimized baseline and after build differ only inside read_records. The
function still converts each candidate to a serde_json::Value, preserving canonical key
ordering and serialization failures. It now serializes candidates into one request-local
Vec<u8>, clearing and reusing that allocation, instead of allocating and growing a new
JSON String for every candidate. Filtering, returned fields, input order,
matching-record offsets, 50-record pages and the existing 512 KiB result error remain
unchanged. Empty queries still skip earlier pages without encoding them. No index,
global cache, input truncation or new production limit was added.

The byte buffer is borrowed as UTF-8 only after a successful default JSON serialization.
A SAFETY comment states that invariant; the locked serde_json 1.0.149 to_string
implementation uses the same unchecked conversion after to_vec. A prototype that
validated the entire buffer again made the Unicode fixture slower, so that redundant
validation was rejected. This does not bypass input validation: original model strings
are valid Rust strings, canonical JSON encoding succeeds first, and the existing
contextual Unicode lowercase search is unchanged.

The buffer is freed when the request returns and retains the largest serialized
candidate capacity until then. Peak memory for very large or heavily
escaped/error-producing records has not been benchmarked. Full Value construction and
Unicode lowercasing still occur on the native foreground thread; the roughly 203 ms
Unicode full-scan floor remains a significant target for later work.

### Fixtures and measurement boundaries

Each native fixture owns 10,001 deterministic records in each of the five domains. Task
descriptions, Calendar notes, Capture bodies and Inbox message bodies contain
substantial ASCII text. Knowledge includes an active body and two saved versions; its
final-match marker exists only in version history. A second Knowledge dataset contains
Greek contextual casing, dotted I, Kelvin sign, accented text and emoji. Queries select
the first 50 records, offset 9,950, only the final record, or no record. Record
construction and connection enablement are outside the timers.

The handler boundary starts at handle_bridge_request and ends at its returned JSON
Value, excluding request construction and validation. The complete boundary starts when
a Python client writes/drains a JSON-RPC request and ends when it receives the full
newline-terminated MCP stdout frame. It includes the actual optimized MCP process,
private authenticated Unix socket, foreground query and response encoding. Parent
response parsing and assertions are outside that timer. The MCP executable is identical
before/after; the native test executable varies. No provider, real user data, browser
rendering, physical input/paint or cold hydration is involved.

There are five warmups and 30 recorded samples per case, for both boundaries and both
run orders. Percentiles use nearest rank (p50/p95/p99 indices 14/28/29 for 30 samples).
All four runs used idle builders and held the native/WASM debug/release profile locks.
Every returned ID, record count and next offset is checked. Complete stdio JSON
fingerprints and frame byte counts agree before/after for all 24 cases, including the
Unicode dataset; the fixture then revokes all five read grants, verifies rejection
through the real socket and drops its private session.

### Full-scan results

Complete MCP path, no-match query. Each cell is p50 / p95 / p99 in milliseconds.

| Domain / data / run order | Before | After |
| --- | ---: | ---: |
| Tasks, ASCII, before → after | 28.238 / 28.718 / 28.824 | 22.979 / 23.586 / 23.655 |
| Calendar, ASCII, before → after | 21.108 / 21.628 / 21.674 | 19.067 / 20.032 / 20.780 |
| Capture, ASCII, before → after | 17.550 / 18.197 / 18.790 | 15.924 / 17.471 / 17.943 |
| Knowledge, ASCII, before → after | 51.693 / 52.080 / 52.944 | 43.943 / 44.751 / 44.899 |
| Inbox, ASCII, before → after | 36.441 / 37.079 / 37.173 | 30.446 / 31.437 / 32.163 |
| Knowledge, Unicode, before → after | 211.723 / 213.912 / 219.527 | 203.612 / 204.538 / 205.482 |
| Tasks, ASCII, after → before | 28.681 / 29.135 / 29.645 | 23.259 / 23.830 / 23.902 |
| Calendar, ASCII, after → before | 21.348 / 21.798 / 21.809 | 16.925 / 17.632 / 17.889 |
| Capture, ASCII, after → before | 17.665 / 17.909 / 17.985 | 14.630 / 16.591 / 16.798 |
| Knowledge, ASCII, after → before | 50.619 / 51.272 / 51.557 | 44.487 / 45.096 / 45.205 |
| Inbox, ASCII, after → before | 36.185 / 36.455 / 36.591 | 28.804 / 29.168 / 29.493 |
| Knowledge, Unicode, after → before | 211.006 / 214.450 / 216.016 | 203.286 / 205.038 / 205.584 |

The ASCII no-match median improves about 9–21% through the complete MCP path across the
five adapters and two run orders. The Unicode Knowledge median improves only about 4%,
from roughly 211–212 ms to 203–204 ms. These are measured host/workload results, not
hardware-independent thresholds. Empty-query page timings vary in both directions; no
unfiltered-page speedup is claimed. All first/deep-page and p99 results remain in the
raw comparison and interactive view.

Native handler only, no-match query:

| Domain / data / run order | Before p50 / p95 / p99 (ms) | After p50 / p95 / p99 (ms) |
| --- | ---: | ---: |
| Tasks, ASCII, before → after | 28.052 / 39.428 / 47.500 | 22.677 / 23.307 / 23.313 |
| Calendar, ASCII, before → after | 20.746 / 21.176 / 21.520 | 18.447 / 20.141 / 20.483 |
| Capture, ASCII, before → after | 17.476 / 18.140 / 18.174 | 16.259 / 17.485 / 17.644 |
| Knowledge, ASCII, before → after | 51.139 / 51.966 / 52.126 | 43.238 / 44.121 / 44.237 |
| Inbox, ASCII, before → after | 36.293 / 37.120 / 37.193 | 29.808 / 31.553 / 32.693 |
| Knowledge, Unicode, before → after | 211.183 / 212.827 / 213.256 | 203.839 / 205.737 / 244.475 |
| Tasks, ASCII, after → before | 28.284 / 29.023 / 29.024 | 22.906 / 23.457 / 23.478 |
| Calendar, ASCII, after → before | 21.105 / 21.583 / 21.644 | 16.787 / 17.446 / 17.792 |
| Capture, ASCII, after → before | 17.273 / 17.593 / 17.632 | 14.670 / 16.560 / 17.177 |
| Knowledge, ASCII, after → before | 50.427 / 51.047 / 51.172 | 44.314 / 45.148 / 45.283 |
| Inbox, ASCII, after → before | 35.888 / 36.546 / 36.603 | 28.460 / 29.826 / 30.175 |
| Knowledge, Unicode, after → before | 209.718 / 210.982 / 211.224 | 203.300 / 204.735 / 205.519 |

Complete MCP path, final-record body/history match:

| Domain / data / run order | Before p50 / p95 / p99 (ms) | After p50 / p95 / p99 (ms) |
| --- | ---: | ---: |
| Tasks, ASCII, before → after | 24.917 / 25.370 / 25.392 | 19.570 / 20.450 / 20.806 |
| Calendar, ASCII, before → after | 21.009 / 21.386 / 21.400 | 18.711 / 19.769 / 20.035 |
| Capture, ASCII, before → after | 17.760 / 18.389 / 18.544 | 15.993 / 17.735 / 19.256 |
| Knowledge, ASCII, before → after | 51.729 / 52.909 / 53.207 | 43.870 / 45.256 / 45.315 |
| Inbox, ASCII, before → after | 36.400 / 36.999 / 37.373 | 30.472 / 31.464 / 31.551 |
| Knowledge, Unicode, before → after | 212.034 / 215.157 / 216.579 | 203.874 / 205.566 / 205.755 |
| Tasks, ASCII, after → before | 25.503 / 26.415 / 26.755 | 19.726 / 20.338 / 20.430 |
| Calendar, ASCII, after → before | 21.309 / 21.963 / 22.080 | 16.996 / 17.437 / 17.728 |
| Capture, ASCII, after → before | 17.610 / 18.106 / 18.227 | 14.960 / 16.859 / 16.992 |
| Knowledge, ASCII, after → before | 50.938 / 51.520 / 51.565 | 44.315 / 45.275 / 45.325 |
| Inbox, ASCII, after → before | 35.973 / 36.873 / 36.964 | 28.668 / 29.538 / 29.761 |
| Knowledge, Unicode, after → before | 211.238 / 213.229 / 248.952 | 203.424 / 206.008 / 238.234 |

### Contracts, prior actions and audit reachability

The native MCP contract suite passes: 111 tests, zero failures, three manual tests
ignored. New tests compare full canonical JSON search against the original
lowercase-string reference across 123 typed records, 15 queries and seven offsets (105
page comparisons), covering reversed struct field declarations, JSON key boundaries,
escapes, nested history, contextual casing and composed/decomposed text. Other
regressions cover a 600,000-byte nonmatching record before short matches, unchanged
result-byte errors, and serialization failures that must abort scanned pages while
leaving skipped/unread pages unencoded. A three-record native smoke test checks actual
bodies/history in all five adapters and Unicode Knowledge.

The first large manual fixture failed because Capture was given a single line: its
constructor projects that line into a bounded title and takes the body from subsequent
lines. The corrected input has an explicit title line and body; the new smoke test
passed before the accepted optimized baseline. The failed executable, source and partial
output are preserved under performance-generic-before-invalid-fixture* and are not used
in the final comparison. An earlier new test also corrected its plain i query to dotted
İ because i legitimately matches the serialized id key. Initial Clippy helper-unwrap
errors were corrected with descriptive expectations. None of these fixture corrections
changed production behavior.

The complete quick gate passes: formatting, native all-target/all-feature Clippy with -D
warnings, WASM Clippy, 2,375 Rust tests (zero failures; 23 manual tests ignored), 36
site tests, site types and benchmark self-checks. The corrected baseline and final
native optimized builds completed in 8m 04s and 8m 00s, with the same two existing
release-only dead-code warnings in preview/settings helpers. No web/model/dependency
production code changed in this loop, so the earlier production WASM build is not
treated as a new build result.

Nine additional optimized runs revalidate earlier finance handler, full stdio and
Unix-socket variants after the shared-dispatch source changed. The preserved
pre-query-optimization binary still takes about 8.89 ms for the final match and
12.64–12.66 ms for no match; the current native handler takes 1.39–1.45 ms and 2.02–2.06
ms. All 32 warm stdio case/order/version fingerprint and byte-count checks match the
previous loop’s recorded payloads. Current-grant rejection and session cleanup pass
again. Removing only the added test module/tests reproduces the earlier source hashes
exactly; outside read_records, the corrected before/after file is byte-for-byte
identical. Historical timings remain historical, with prior hashes and fresh review
receipts preserved.

A reachability review also corrects the pending standalone variants: the current MCP
executable creates an empty AppState and exposes no standalone tool that creates threads
or starts background tasks. Its thread/task reads cannot reach populated maps through
the current catalog. Dry-run writes do create local workflows/previews, so growing
workflow/audit history remains a real performance target. This removes an invented data
state from the audit, without reducing the goal of covering every reachable action,
argument, error and lifecycle variant.

The source inventory remains 94 HTTP entries, 86 palette entries with 86 dispatch links,
19 views, 9 slash commands, 32 tools, four MCP methods, 10 workflow operations and 1,036
binding references. It now has 13 reviewed partial-evidence entries across 12 catalogue
identities. All active listed source hashes are current after the explicit prior-action
revalidation. This is not a completion percentage or a transitive call-graph proof.

### Artifacts and reproduction

| Executable | SHA-256 |
| --- | --- |
| Corrected native before | `9483aa798548cbb5d5a0cdc50c0fcd09f02f1ca7ad2ac4f7a6fc8eb8edee9a09` |
| Native after | `69c0f4c1d6c7ec77ece74eed35d7dc70858c091abf63d2f413da01303f828fb3` |
| Fixed MCP executable | `abf04b583af350627454466ca3710b1e081cfbcd0f2cb4ec15d8be4485c30961` |

Raw data is under target/: performance-generic-{before,after}-{forward,reverse}.log
contains the handler samples; the corresponding .ascii.json and .unicode-knowledge.json
files contain full-stdio samples, response fingerprints and frame sizes. The four
*-run.json receipts record executable hashes, timestamps, exact commands, successful
exits and profile locks. performance-generic-comparison.json combines all cases and 50
listed source hashes. performance-generic-change-scope.json pins the sole
production-function delta. performance-generic-before-source/ preserves the corrected
baseline source, and the initial failed fixture has a separate archive.

Gate details are in performance-gate-pass16/ and performance-generic-gate.log.
Earlier-action receipts are performance-generic-revalidation-runs.json and
performance-generic-prior-evidence-revalidation.json;
performance-generic-prior-evidence-ledger.json preserves the previous ledger before
source repinning. performance-mcp-reachability.json records the standalone state review.
Prototype sources/build logs and both validating/nonvalidating UTF-8 benchmark logs are
kept as performance-generic-prototype*. The source pins cover only the named files, not
every transitive runtime dependency.

To reproduce the current native run, build the optimized MCP executable and native test
binary, then run the printed test executable directly after all builds finish:

```sh
cargo build -p bruhs-mcp --release --locked
cargo test -p bruhs-desktop --bin bruhs-desktop --release --locked --no-run
BRUHS_PERF_MCP_BIN="$PWD/target/release/bruhs-mcp" \
BRUHS_PERF_GENERIC_OUTPUT="$PWD/target/performance-current-generic.json" \
target/release/deps/bruhs_desktop-eb5d5e922fc6f198 \
  generic_native_read_action_cost --ignored --nocapture
```

Use the executable path printed by Cargo if its suffix differs. The fixture supplies its
own private workspace connection and writes separate .ascii.json and
.unicode-knowledge.json outputs. It invokes scripts/performance-mcp-record-reads.py,
which reuses the existing stdio transport harness. Do not point these deterministic
synthetic assertions at a real user workspace. The preserved performance-generic-run.py
driver adds the before/after run-order and profile-lock controls.

Remaining work includes the foreground cost of Value construction and contextual Unicode
lowercasing, cheap isolated snapshots for background scans/indexing, record
construction/import throughput, peak memory and latency for very
large/escaped/error-producing records, Unicode data in the other four domains,
concurrent requests/cancellation/disconnection, cold loading, and actual input-to-paint
behavior. The current 10,001-record full scans still occupy the UI thread for tens to
hundreds of milliseconds despite the allocation improvement.

## Unicode record search with exact lowercase semantics

The next native-read profile found contextual Unicode lowercasing dominating the
10,001-record Knowledge scan. During a separate two-second sample, 1,198 of the 1,490
samples inside the test foreground handler were under `str::to_lowercase` (about 80%).
This ratio describes that busy handler thread; the OS main thread and Tokio workers were
mostly waiting. It is not whole-app CPU utilization. Profiled runs are excluded from all
accepted latency comparisons below. The final after-build profile records 1,498 handler
stack samples with no `str::to_lowercase` frames during the same ASCII-query scan window.
This does not cover the unchanged non-ASCII-needle path. The profile counts and binary
hashes are preserved in `target/performance-unicode-profile-review.json`.

The shared private `LowercaseQuery` now searches ASCII needles without lowercasing every
Unicode field. It still implements the exact existing
`field.to_lowercase().contains(query.to_lowercase())` behavior. Exhaustive enumeration of
the pinned Rust 1.95 / Unicode 17 scalar mappings finds two non-ASCII sources of ASCII:
`İ` → `i` plus combining dot, and `K` → `k`. Other non-ASCII casing cannot create, remove
or bridge an ASCII substring. When a relevant source is present, a single byte-scan pass
normalizes those two complete UTF-8 encodings into a temporary string, then performs the
ASCII search. A relevant one-byte `i` or `k` query succeeds immediately. Irrelevant Unicode
stays untouched. Non-ASCII needles continue to use standard whole-string lowercasing,
including contextual Greek final sigma. The implementation follows the
[Rust 1.95 standard-library lowercasing code](https://github.com/rust-lang/rust/blob/1.95.0/library/alloc/src/str.rs#L380-L425).
It does not add case folding, Unicode normalization, a cache, a search limit, or a change
to searchable JSON fields.

The normalizer verifies complete source encodings after vectorized final-byte searches;
unrelated UTF-8 suffixes cannot be mistaken for dotted I or Kelvin. The existing
non-overlapping ASCII candidate scan and overlapping-prefix fallback remain in use.
Exhaustive tests also establish the ASCII-match byte-length bound used to reject short
fields before scanning. Release prototypes showed that explicitly inlining the matcher
and ASCII helper while keeping normalization out of line preserves ASCII throughput.
These prototype timings informed implementation choice; accepted results come from the
actual native handler and full transport below.

Independent finance revalidation caught an intermediate regression before acceptance.
The larger matcher made empty-query deep pages slower: full stdio p50 rose from about
0.38 to 0.72 ms in both orders. That implementation and its raw measurements are preserved
under `target/performance-unicode-initial/`. Finance now bypasses the predicate for an
empty query and skips on the source iterator before projection. Filtered pages skip
matching records before projection, with offsets still relative to matches. The original
bounded encoder, privacy gates, projected fields, amount redaction, errors, record order
and pagination metadata remain unchanged. Source reconstruction checks verify the exact
scope of these changes. New contracts fail if an unfiltered page invokes the predicate or
projects skipped records, and verify filtered resume offsets.

### Optimized action comparisons

The baseline is the prior pass's accepted optimized native binary, with its sources
preserved under `target/performance-unicode-before-source/`. The same optimized MCP
executable is used on both sides. All builds and the gate finish before timing; four
Cargo-profile locks prevent overlapping builds. Runs execute before/after and then
after/before, with private synthetic workspaces and connections. Each generic case has
five warmups and 30 measured samples. Percentiles use nearest rank; p99 is the largest of
30 samples. Complete stdio JSON fingerprints and frame sizes match for every paired
case. Native checks verify exact IDs, record counts, pagination, grant revocation and
cleanup. Timing never includes fixture construction or response validation.

The complete stdio boundary starts at parent request write/drain and ends when the entire
MCP stdout frame arrives, including the actual MCP process, private Unix socket and
foreground native handler. Knowledge contains 10,001 records, repeated Greek/Latin/emoji
text and two saved versions. The final marker occurs only in version history, preserving
full canonical JSON search. Values below are **p50 / p95 milliseconds**.

| Case | Before → after order: before | After | After → before order: before | After |
|---|---:|---:|---:|---:|
| First 50, empty query | 1.630 / 1.959 | 1.622 / 1.986 | 1.602 / 1.863 | 1.565 / 1.987 |
| Deep page at 9,950, empty query | 1.618 / 2.072 | 1.678 / 1.875 | 1.622 / 1.893 | 1.640 / 1.922 |
| Final match only in saved version history | 203.968 / 205.031 | 39.675 / 40.822 | 203.930 / 206.437 | 39.697 / 40.169 |
| ASCII no-match scan | 204.445 / 205.981 | 39.688 / 40.499 | 206.583 / 242.389 | 39.649 / 40.298 |
| ASCII `i`, first-page hit | 2.545 / 3.336 | 1.734 / 1.981 | 2.613 / 2.906 | 1.700 / 2.100 |
| ASCII `k`, first-page hit | 2.647 / 2.902 | 1.638 / 1.895 | 2.653 / 2.915 | 1.735 / 1.968 |
| Final sigma `ς`, first-page hit | 2.643 / 2.935 | 2.669 / 2.947 | 2.574 / 2.942 | 2.594 / 2.922 |
| Nonfinal sigma `Σ`, no match | 204.807 / 206.373 | 202.626 / 205.589 | 204.056 / 209.017 | 203.407 / 204.348 |
| Unicode `ζ-unseen`, no match | 204.931 / 206.380 | 202.960 / 204.147 | 204.442 / 208.950 | 203.512 / 213.828 |
| ASCII `missing-kelvin`, no match | 204.307 / 206.057 | 63.503 / 64.279 | 200.808 / 217.071 | 63.695 / 64.498 |

The native handler boundary measures request-handler entry through returned JSON Value;
request construction is outside the timer. It retains the four original cases:

| Case | Before → after order: before | After | After → before order: before | After |
|---|---:|---:|---:|---:|
| First 50, empty query | 0.275 / 0.427 | 0.279 / 0.297 | 0.288 / 0.356 | 0.279 / 0.340 |
| Deep page at 9,950, empty query | 0.282 / 0.313 | 0.288 / 0.331 | 0.298 / 0.322 | 0.299 / 0.402 |
| Final match only in saved version history | 204.239 / 207.794 | 39.377 / 40.144 | 204.182 / 208.318 | 39.240 / 41.727 |
| ASCII no-match scan | 203.583 / 205.417 | 39.290 / 40.901 | 203.572 / 206.150 | 39.645 / 40.311 |

The large ASCII scans over Unicode-heavy records improve in both orders. Dotted-I and
Kelvin first-page hits also improve, and the ambiguous ASCII miss keeps exact expansion
boundaries while avoiding full Unicode casing. Non-ASCII misses still cost roughly
200 ms; their tails remain variable. Empty pages and final-sigma first-page hits
are small and have mixed/noisy tails. No general Unicode-query or whole-app speedup is
claimed.

The same fixture also measures all four standard cases in Tasks, Calendar, Capture,
Knowledge and Inbox with ASCII data. Their full-scan medians stay in the prior range;
small movements and isolated tails are retained in the raw data and interactive chart.
Serialization and owned JSON construction remain the dominant cost in those generic
scans. The page fix does not replace generic full-JSON search with projected metadata.

### Finance regression check and corrected paging

The real finance fixture uses 100,001 synthetic ASCII transactions. The handler has
five warmups and 50 samples, preserving its existing rounded p50/p95 calculation; stdio
has ten warmups and 200 samples with nearest-rank percentiles. Native before/after uses
the same baseline as the Unicode comparison. Finance results are also preserved in
`target/performance-unicode-finance-comparison.json`. Both orders run with the same MCP binary.
Values are **p50 / p95 milliseconds**; differences between these boundaries must not be
subtracted as transport overhead.

| Boundary / case | Forward: before | After | Reverse: before | After |
|---|---:|---:|---:|---:|
| handler / First page | 0.067 / 0.077 | 0.067 / 0.069 | 0.068 / 0.069 | 0.067 / 0.070 |
| handler / Deep page at 99,950 | 0.156 / 0.161 | 0.067 / 0.069 | 0.158 / 0.160 | 0.067 / 0.082 |
| handler / Last match | 1.324 / 1.572 | 0.583 / 0.751 | 1.313 / 1.486 | 0.586 / 0.620 |
| handler / No match | 2.031 / 2.254 | 1.898 / 2.015 | 1.962 / 2.100 | 2.012 / 2.116 |
| stdio / First page | 0.274 / 0.335 | 0.271 / 0.300 | 0.272 / 0.327 | 0.272 / 0.308 |
| stdio / Deep page at 99,950 | 0.374 / 0.456 | 0.272 / 0.306 | 0.359 / 0.380 | 0.272 / 0.324 |
| stdio / Last match | 1.597 / 1.746 | 0.887 / 0.987 | 1.535 / 1.684 | 0.886 / 1.004 |
| stdio / No match | 2.220 / 2.400 | 2.089 / 2.287 | 2.156 / 2.334 | 2.106 / 2.301 |

The corrected deep page now has the same handler cost as the first page, and its
stdio cost is lower than the original baseline in both orders. Final-match scans also
improve; no-match handler results are mixed at this scale. The intermediate regression
and its timings remain archived. Eleven optimized runs cover both finance handler orders, both full-stdio
orders, two additional old-MCP variants on the new native binary, and native IPC with
revocation and cleanup. Forty-eight complete warm finance/discovery/error payloads and
frame sizes match phase 15; 96 generic page payloads and sizes match phase 16. Startup
samples stay separate and are not attributed to this change.

### Verification, provenance and remaining work

- Eight matcher tests pass, including all Unicode scalar mappings, the ASCII length
  bound, contextual sigma, dotted-I expansion, Kelvin, escaped/literal text, long
  overlapping queries, 68,000 generated multiscript query/field comparisons and unrelated
  UTF-8 suffix collisions. These tests compare to the standard-library lowercase oracle.
- All 117 native bridge contracts pass; three manual timing fixtures are ignored by the
  ordinary test command. Full-page finance behavior, redaction, permissions, byte limits,
  errors and unchanged records remain covered.
- The final quick gate passes formatting, native all-feature/all-target and WASM Clippy
  with warnings denied, **2,381 Rust tests with zero failures**, 23 manual ignores, **36
  site tests**, site types and benchmark self-checks. The optimized native test build also
  succeeds; its two pre-existing release-only unused-item warnings remain recorded.
- The reviewed ledger has 14 partial entries across 12 action/method identities. Older
  evidence is explicitly revalidated and source-repinned while historical timings remain
  historical. This is not a completion percentage or whole-action acceptance.

The final native SHA-256 is `b20021a6156aeb4de5147386f266d70ff7a989874b6adce5db7ac3f66dbdffec`;
the baseline is `69c0f4c1d6c7ec77ece74eed35d7dc70858c091abf63d2f413da01303f828fb3`. The fixed MCP binary is
`abf04b583af350627454466ca3710b1e081cfbcd0f2cb4ec15d8be4485c30961`.
`target/performance-unicode-comparison.json` pins the 50 named sources and combines all
24 handler and 30 full-stdio cases in both orders. Raw samples are in the corresponding
`performance-unicode-{before,after}-{forward,reverse}.log`, `.ascii.json` and
`.unicode-knowledge.json` files; `*-run.json` records exact commands, timestamps, profile
locks, hashes and successful exits. `performance-unicode-change-scope.json` verifies the
finance projections and original page encoding body. `performance-unicode-revalidation-runs.json`
and `performance-unicode-prior-evidence-revalidation.json` preserve cross-phase checks;
`performance-unicode-prior-evidence-ledger.json` preserves the old ledger. Gate details
are in `performance-gate-pass17/` and `performance-unicode-gate.log`. Prototype sources,
logs, Unicode-property checks and separate CPU profiles remain under the same prefix.
The source pins cover listed files, not every transitive dependency or runtime condition.

The previous generic-reader reproduction command still applies; its dynamic Python
harness now adds the six Unicode controls. No real user records, provider calls,
connection enablement, physical UI input or paint enter these measurements. The next
read optimization needs to address the remaining non-ASCII 200 ms scans, 40–70 ms
foreground ASCII scans over large Unicode JSON, serialization/Value ownership, cheap
isolated snapshots, cold hydration, concurrent clients, cancellation, error latency and
memory. Unicode-heavy cases in the other domains, other projected sections (including large offsets in Health and meeting summaries),
reachable standalone preview/audit growth, provider/capture/sync actions and physical UI behavior
remain unaudited variants. The overall goal remains active.

## Projected native reads and direct deep paging

The next pass extends actual handler and full-stdio measurements to 18 projected read
variants: all five Finance sections in both amount-visibility modes, four Health
sections, Meeting identities, local note identities, People discovery and Chat summaries.
The baseline showed extra empty-query deep-page work in all five remaining reader
implementations. For example, 100,001 Meeting identities required walking preceding
records even though the requested response contained only 50 identities. Chat summaries
also read preceding GPUI chat entities before reaching the requested page.

Those readers now use the existing `read_matching_page` helper. Empty queries skip on
the source iterator before projection; they do not construct a predicate result for each
preceding record. Nonempty queries retain their metadata predicates, skip matching
records and project the selected page. Chat reads only the selected entities for an
empty query; nonempty matching still reads candidate entities, with projection reads
bounded to the selected page and lookahead. The helper, bounded encoder, Unicode query
matcher, Finance reader, dispatcher, grants, query/offset validators and exact-ID
context/history functions remain unchanged. No cache, index, production limit or new
search field is introduced.

The source-scope check verifies that the changes stay within five listing methods and
their helper imports. All exterior code, including detail/history methods and projection
types, is byte-identical; validators, match fields, aliases, struct projections and
response metadata are checked separately. The complete Health file is reproduced by
replacing only its four paging wrappers with the existing helper and identity projection.

### Coverage and measurement boundary

The reproducible fixture stages owned collections in three groups. Each non-Chat
section has **100,001** source records; Chat has **10,001 distinct GPUI chat entities**.
Finance's two visibility modes reuse the same records. Public labels contain Greek,
accents, dotted I and Kelvin where their models have labels; Sleep and Recovery search
IDs only. Alternating IDs match a selective query, and the last ID contains a separate
marker. Supported models include representative hidden first-record content: transaction
and recovery notes, Meeting summary/audio metadata, note body, People observations/voice
fingerprint and Chat message history. These fields keep their existing search and
visibility rules.

Every variant has seven cases: first page, empty-query deep page, final-ID match, absent
ASCII query, selective deep page, selective final page and a body-marker query that is
absent from searchable metadata. Deep unfiltered offsets are 99,950 (9,950 for Chat).
Selective offsets are 49,950 (4,950 for Chat), followed by a final-page resume check. IDs,
record counts, next offsets, Finance redaction and excluded discovery content are checked.
The hidden marker is present only where the fixture's model supports such content; other
sections exercise absence of that same query.

Two pre-change small runs and a post-change small run use 123 records per variant and
check **126 complete MCP JSON fingerprints and frame sizes**, including the actual MCP
process and private Unix socket. Small runs use one warmup and two samples and are
functional checks only. Their timings are excluded from the performance comparison.

The optimized baseline adds only test fixtures to the previous production sources. Its
60 named source inputs are preserved under `target/performance-projected-before-source/`.
The after build changes the five listing readers. The same optimized MCP executable is
used throughout. Each run starts after builders stop, with four Cargo-profile locks held.
The repeat series starts after both binaries exist. Each series runs before/after and
then after/before, with five warmups and 30 measured samples per case. The primary
summaries pool all **60 samples per binary
and order**; both original 30-sample series stay separately inspectable. Percentiles use
nearest rank, with p99 the largest of 60. All **eight** large runs preserve complete JSON
fingerprints and frame sizes for **126 handler cases and 126 full-stdio cases per run**.

Handler timing starts at native request-handler entry and ends at returned JSON Value.
Request construction and result assertions are outside the timer. Full stdio timing
starts at parent request pipe write/drain and ends at the complete MCP stdout frame,
including the MCP child, real private Unix socket and foreground native reader. Parent
JSON validation is outside that timer. The two boundaries must not be subtracted to
infer transport overhead. Fixture construction, GPUI entity mounting, cold hydration,
provider work, connection enablement, concurrent clients and physical input/paint are
outside these measurements.

### Repeated series and rejected inlining

The repeated series investigates variation in nonempty queries and unchanged Finance
controls, plus an isolated People stdio burst. Every sample from both series is retained
and pooled; the interactive comparison can also show either series alone. Nonempty
query timings remain variable, including shifts in unchanged code. These measurements
do not assign those shifts to a proven cause or establish a general scan speedup.

A separate experiment explicitly inlined the field-slice matcher. Its small release
prototype improved selective Budget scans about 5%, but full native and legacy ASCII
Finance comparisons did not show a consistent whole-action gain. That hint was removed.
All experiment binaries, sources, raw comparisons and checks remain under
`target/performance-projected-inline-experiment/`. The accepted source and matching
optimized binary exactly reproduce the previously gated five-reader candidate. The
Unicode matcher, Finance reader and shared encoder retain their previous source hashes.
The first candidate archive is `target/performance-projected-paging-candidate/`.

### Paired optimized results

Empty-query deep pages below return 50 records. Values are **p50 / p95 milliseconds**.
The 100,001-record variants use offset 99,950; Chat uses offset 9,950.

Native handler:

| Variant | Forward: before | After | Reverse: before | After |
|---|---:|---:|---:|---:|
| Health workouts | 0.255 / 0.274 | 0.062 / 0.096 | 0.255 / 0.285 | 0.061 / 0.071 |
| Health sleep | 0.211 / 0.227 | 0.049 / 0.050 | 0.213 / 0.260 | 0.052 / 0.074 |
| Health recovery | 0.234 / 0.256 | 0.058 / 0.061 | 0.234 / 0.260 | 0.057 / 0.061 |
| Health habits | 0.260 / 0.300 | 0.061 / 0.062 | 0.260 / 0.316 | 0.064 / 0.080 |
| Meeting identities | 0.716 / 1.048 | 0.081 / 0.095 | 0.626 / 0.977 | 0.079 / 0.080 |
| Local note identities | 0.252 / 0.318 | 0.041 / 0.042 | 0.249 / 0.328 | 0.042 / 0.046 |
| People discovery | 0.134 / 0.141 | 0.078 / 0.084 | 0.135 / 0.136 | 0.075 / 0.079 |
| Chat summaries | 0.227 / 0.314 | 0.077 / 0.081 | 0.227 / 0.296 | 0.078 / 0.108 |

Complete MCP stdio:

| Variant | Forward: before | After | Reverse: before | After |
|---|---:|---:|---:|---:|
| Health workouts | 0.473 / 0.551 | 0.282 / 0.377 | 0.494 / 0.612 | 0.272 / 0.373 |
| Health sleep | 0.412 / 0.501 | 0.240 / 0.339 | 0.426 / 0.497 | 0.247 / 0.326 |
| Health recovery | 0.451 / 0.532 | 0.254 / 0.389 | 0.470 / 0.542 | 0.254 / 0.357 |
| Health habits | 0.485 / 0.545 | 0.269 / 0.320 | 0.485 / 0.631 | 0.271 / 0.349 |
| Meeting identities | 1.096 / 1.481 | 0.366 / 0.440 | 1.202 / 1.588 | 0.312 / 0.365 |
| Local note identities | 0.449 / 0.537 | 0.235 / 0.349 | 0.468 / 0.641 | 0.233 / 0.313 |
| People discovery | 0.362 / 0.417 | 0.477 / 1.864 | 0.375 / 0.430 | 0.313 / 0.421 |
| Chat summaries | 0.532 / 0.732 | 0.312 / 0.400 | 0.547 / 0.764 | 0.322 / 0.445 |

The selective deep-page control retains linear predicate work and match-relative
pagination. It verifies the other branch of the helper rather than treating every query
as an unfiltered page. Native handler values are **p50 / p95 milliseconds**:

| Variant | Forward: before | After | Reverse: before | After |
|---|---:|---:|---:|---:|
| Health workouts | 3.457 / 4.546 | 3.478 / 5.702 | 3.655 / 4.476 | 3.551 / 4.367 |
| Health sleep | 1.360 / 3.697 | 1.470 / 4.160 | 1.310 / 1.448 | 1.314 / 1.445 |
| Health recovery | 1.383 / 1.496 | 1.834 / 4.567 | 1.399 / 1.531 | 1.312 / 1.491 |
| Health habits | 1.863 / 2.143 | 1.906 / 2.121 | 1.873 / 2.040 | 1.946 / 2.144 |
| Meeting identities | 2.551 / 3.468 | 2.488 / 3.084 | 2.561 / 3.730 | 2.467 / 3.472 |
| Local note identities | 1.779 / 1.923 | 1.757 / 1.871 | 1.751 / 1.994 | 1.726 / 1.915 |
| People discovery | 2.189 / 2.454 | 2.362 / 7.298 | 2.286 / 2.482 | 2.152 / 2.384 |
| Chat summaries | 0.559 / 0.836 | 0.501 / 0.551 | 0.549 / 0.767 | 0.514 / 0.846 |

First pages, final matches, misses, filtered final pages, hidden-marker controls and all
ten Finance section/visibility combinations are retained in the comparison artifact and
interactive chart. Small movements and noisy tails remain visible; these results do not
establish a speedup for every case or a whole-application latency bound.

### Verification and evidence

- **118 native bridge contracts pass**, with four manual timing fixtures ignored by the
  normal command. Existing permission, redaction, validation, byte-limit, error, saved
  context, pagination and unchanged-record contracts still pass. The timing fixture also
  checks that reads return errors after grant revocation and cleans up its private session.
- The full quick gate passes formatting, strict native all-feature/all-target Clippy,
  WASM Clippy, **2,382 Rust tests with zero failures**, 24 manual ignores, **36 site
  tests**, site types and benchmark self-checks. Both optimized native builds succeed;
  their two known release-only unused-item warnings are retained in build logs.
- Five additional optimized prior-fixture runs cover generic reads, Finance handler,
  Finance through both old/new MCP variants, and native IPC. All **30 generic** and **16
  Finance/discovery/error** warm response fingerprints and frame sizes match phase 17.
  These are after-only revalidations; historical timings stay historical.
- The reviewed ledger now has **20 partial entries across 16 action/method identities**,
  including six native groups added in this pass. Older entries are explicitly revalidated
  and source-repinned. This remains partial variant evidence, not whole-action acceptance
  or a completion percentage.

The baseline native SHA-256 is `18255d224a834ff57fdd0dae149e7840bcbad7d5a523050d1c975af1ef3c2f1f`;
the after build is `a38d96a651db42ea74d267a138d9143d22076fad8bc3750de3e287d0f2169a21`. Both use MCP
`abf04b583af350627454466ca3710b1e081cfbcd0f2cb4ec15d8be4485c30961`.
`target/performance-projected-comparison.json` combines both boundaries/orders and pins
60 named sources. Each `performance-projected-{before,after}-{forward,reverse}.log`
contains direct samples; the 18 corresponding `.VARIANT.json` files contain full-stdio
samples and fingerprints. The four `*-run.json` receipts record exact commands,
timestamps, executable hashes, output files, successful exits and profile locks.
`performance-projected-change-scope.json` records source invariants;
`performance-projected-smoke-receipt.json` and `performance-projected-smoke-after-receipt.json`
record small real-stdio equivalence. `performance-projected-revalidation-runs.json` and
`performance-projected-prior-evidence-revalidation.json` preserve cross-phase checks;
`performance-projected-prior-evidence-ledger.json` preserves the old ledger. Gate details
are under `performance-gate-pass18/` and `performance-projected-gate.log`. The pooled artifact records both series and their eight run receipts;
`performance-projected-initial-comparison.json` and `performance-projected-repeat-comparison.json`
preserve their separate summaries. Source pins cover named files, not every transitive dependency or runtime condition.

To reproduce a current full run after building the optimized MCP and native test binaries:

```sh
BRUHS_PERF_MCP_BIN="$PWD/target/release/bruhs-mcp" \
BRUHS_PERF_PROJECTED_OUTPUT="$PWD/target/performance-current-projected.json" \
target/release/deps/bruhs_desktop-eb5d5e922fc6f198 \
  projected_native_read_action_cost --ignored --nocapture
```

Use the executable path Cargo prints if its suffix differs. Leave
`BRUHS_PERF_PROJECTED_SMOKE` unset for full measurements. The native fixture supplies
its own private connection, invokes `scripts/performance-mcp-projected-reads.py`, and
writes one output per variant. These deterministic assertions target synthetic fixtures,
not a real user workspace.

Nonempty metadata searches still scan candidates. Exact-ID details and large completed
message, transcript, observation and relationship histories remain separate paths.
Other record sizes/distributions, stable attribution of small scan-timing changes,
byte-limit/error timing, cold loading, concurrency,
cancellation, physical UI latency and peak memory remain unverified variants. The
Unicode-heavy full-JSON scans from the prior pass, reachable standalone preview/audit
history, capture/provider/sync work and the wider UI action catalogue also remain in the
active goal.

## Standalone workflow history and audit serialization

This pass measures the reachable standalone preview and audit paths as their
process-local history grows. A newly created workflow previously required an ID scan
from the start of the workflow vector before previewing it. Audit reads built one
complete intermediate JSON Value tree, then pretty-printed it. Every `tools/call`
response also copied its owned result text while converting the result into a Value.

The new `AutomationState::create_workflow_preview` uses the position just appended by
the existing creation method. It delegates to the same preview checks and transitions;
public ID-based previews retain first-match lookup, including duplicate IDs. Jobs are
still checked before previewing. This removes the new-workflow lookup scan without a
cache or persisted index. It does not promise constant-time preview when jobs or
operations grow, and it does not change native preview/approval/apply/undo behavior.

Audit encoding now pretty-prints canonical rows while keeping one row's intermediate
Value tree live. The top-level key layout and each row still follow the previous
`json!` serialization, including nested fields. The response still includes the complete
permissions, workflows, jobs and audit arrays in the same order. The complete output
String and final stdio frame still grow with history; this is not bounded overall
memory or zero-allocation serialization. The same automation mutex is held through
encoding. The result text is then moved into the response Value, preserving optional
`isError`, all text blocks and their content. No production output cap is introduced.

### Reachability, fixtures and boundary

The fixture starts a real standalone MCP executable in a fresh process for each seed
history: **0, 128, 1,024 and 10,000** workflows. It seeds history only through valid
Task dry runs. It then reads the audit, previews one valid operation in each of ten
domains, and exercises rejected apply, invalid operation, empty Chat, empty thread/task
lists, ping and complete tool discovery: **18 cases per history**. Inputs contain Greek,
accents, emoji, quotes, backslashes, newlines and a literal timestamp-field name in user
text. All writes request dry run and remain blocked by default Write grants. No domain
records, provider requests, messages or jobs are created. AppState construction does
not spawn its provider background work; the default meeting store is in-memory.

Each preview case adds five warmups plus 30 measured workflows. Thus the ten domain
cases span seed N through N+350; their exact starting and ending histories are recorded.
Audit timing runs first, at exactly N workflows. The final audit checks every workflow,
operation, pending step, blocked domain, correlated audit event, permission and empty
job array. Rejected/invalid controls leave those counts unchanged. The current standalone
catalog cannot populate its thread/task maps, so those read controls remain empty.

Both optimized MCP binaries are fixed before the accepted comparison. Four runs execute
in before/after/after/before order, with builders stopped and four Cargo profile locks
held. Each case retains all **30 samples per binary and order**, following five warmups.
The combined chart view pools all 60 samples per binary; separate orders remain
available. Percentiles use nearest rank. History sizes run in ascending order inside
each binary run; binary run order does not reverse that fixture order.

The timer starts at parent request-pipe write/drain and ends at the complete stdout
newline frame. Parent response parsing, state validation and normalization occur after
the timer. All **144 case/order before/after comparisons** preserve normalized result
text hashes, raw response sizes and exact history ranges; the success/error envelopes
and complete JSON-RPC frame are validated. Only validated UUIDs and their known
13-digit timestamp fields are normalized. User content, whitespace, key ordering,
array ordering and every other result text byte remain part of the check. Additional
contracts compare exact pretty JSON bytes and complete tool-result encoding, including
`isError: false`, multiple blocks and large owned-text allocation reuse.

These are warm serial full-stdio measurements. Startup, cold hydration, concurrent
clients, slow readers, cancellation, granted native executions, providers and physical
input/paint are outside the comparison. The benchmark's 64 MiB line-reader limit only
allows it to collect complete large responses; it is not a new server limit.

### Paired optimized results

Complete audit reads, **p50 / p95 milliseconds**:

| Seeded workflows | Response bytes | Forward: before | After | Reverse: before | After |
|---|---:|---:|---:|---:|---:|
| 0 | 847 | 0.108 / 0.183 | 0.095 / 0.182 | 0.075 / 0.104 | 0.092 / 0.114 |
| 128 | 109,543 | 0.475 / 0.557 | 0.500 / 0.589 | 0.489 / 0.605 | 0.520 / 0.614 |
| 1,024 | 871,167 | 3.137 / 3.398 | 2.902 / 3.156 | 3.139 / 3.474 | 2.903 / 3.291 |
| 10,000 | 8,509,743 | 32.077 / 33.293 | 31.233 / 32.585 | 32.479 / 33.711 | 31.219 / 32.109 |

At 1,024 workflows the audit median improves about 7.5% in both orders; at 10,000 it
improves about 2.6% and 3.9%. At 128 rows it becomes **5.3% and 6.3% slower** (about
25–31 microseconds); the empty-history direction is mixed. The tradeoff remains
visible. The 10,000-workflow response is still roughly 8.5 MB and takes about 31 ms.
This is a modest full-transport improvement, not an isolated serializer speedup claim.

Previews in the 10,000-seed process, **p50 / p95 milliseconds**. Each case starts at
the history below and adds another 35 workflows:

| Domain | History at case start | Forward: before | After | Reverse: before | After |
|---|---:|---:|---:|---:|---:|
| Chat | 10,000 | 0.108 / 0.154 | 0.095 / 0.143 | 0.115 / 0.144 | 0.109 / 0.157 |
| Tasks | 10,035 | 0.132 / 0.210 | 0.090 / 0.152 | 0.114 / 0.125 | 0.094 / 0.142 |
| Calendar | 10,070 | 0.111 / 0.131 | 0.088 / 0.103 | 0.121 / 0.137 | 0.087 / 0.093 |
| Meetings | 10,105 | 0.109 / 0.119 | 0.089 / 0.095 | 0.119 / 0.202 | 0.096 / 0.132 |
| Knowledge | 10,140 | 0.110 / 0.122 | 0.090 / 0.109 | 0.114 / 0.130 | 0.091 / 0.103 |
| Inbox | 10,175 | 0.116 / 0.121 | 0.086 / 0.091 | 0.113 / 0.151 | 0.088 / 0.145 |
| People | 10,210 | 0.114 / 0.140 | 0.088 / 0.101 | 0.123 / 0.197 | 0.091 / 0.099 |
| Finance | 10,245 | 0.109 / 0.122 | 0.088 / 0.103 | 0.114 / 0.133 | 0.091 / 0.150 |
| Health | 10,280 | 0.111 / 0.120 | 0.100 / 0.226 | 0.115 / 0.126 | 0.087 / 0.096 |
| Capture | 10,315 | 0.117 / 0.187 | 0.087 / 0.092 | 0.116 / 0.214 | 0.091 / 0.103 |

All ten large-history preview medians improve in both orders. Smaller histories have
mixed movements and tails; all four sizes, all ten domains and all controls remain in
the comparison artifact. The comparison includes the known-position lookup and owned
result conversion together; it does not assign their separate latency contributions.

The fixture also records macOS `/usr/bin/time -l` counters for the **entire MCP process**,
including seeding, all cases and final checks. Maximum RSS below is **MiB**; the local
`getrusage(2)` documentation defines its raw counter in bytes:

| Seeded workflows | Forward: before | After | Reverse: before | After |
|---|---:|---:|---:|---:|
| 0 | 10.2 | 9.0 | 10.3 | 9.0 |
| 128 | 11.2 | 9.8 | 11.6 | 9.8 |
| 1,024 | 21.0 | 14.5 | 20.8 | 14.5 |
| 10,000 | 564.6 | 533.1 | 565.8 | 533.1 |

These are two whole-process observations per version, not per-action memory samples
or a p95. Parent Python memory is excluded. Raw peak-footprint, CPU, page-reclaim and
other counters remain in the artifacts without attributing them to a proven cause.
The large retained footprint remains a profiling target. The output String/frame and
stored history remain proportional to the complete response.

### Verification and evidence

- The full quick gate passes formatting, strict native all-target/all-feature Clippy,
  WASM Clippy, **2,388 Rust tests with zero failures**, 24 manual ignores, **36 site
  tests**, site types and benchmark self-checks. The MCP/models targeted run also
  passes, and the optimized MCP executable builds successfully.
- A source guard checks **1,048 existing Rust/config/harness inputs**. Four production
  files change, the other 1,044 are identical, and the audit module is the only added
  production Rust file. Model declarations, permissions, creation/job/apply/undo
  methods, metadata readers, page/query helpers, schemas and prior tests are unchanged.
  The extracted preview body is reproduced exactly after normalizing indexed access
  and the equivalent ID clone. Six new contracts cover the changed behavior.
- Four after-only compatibility runs check **180 prior full-stdio fingerprints and
  sizes**: 30 generic native, 126 projected native, eight legacy native Finance/error/
  discovery controls and 16 original standalone/synthetic-bridge controls. They use
  the new MCP and the fixed accepted native fixture executable. That fixture predates
  the shared-model preview helper, while native read sources are unchanged. Current
  model lifecycle behavior is covered by the rebuilt full workspace gate. These
  checks do not create a new current-native timing comparison.
- The reviewed ledger has **32 partial entries across 27 action/method identities**,
  adding ten standalone preview variants, growing complete audit output and owned
  `tools/call` text transfer. Nineteen source-affected older entries are explicitly
  revalidated and source-repinned; the unchanged native Finance entry retains its
  original evidence. Historical timings keep their original executable identities.
  No action is promoted to all-variants acceptance or a completion percentage.

The before MCP SHA-256 is `abf04b583af350627454466ca3710b1e081cfbcd0f2cb4ec15d8be4485c30961`;
the after MCP is `e2b76d184c6b462a2b0f09f0eecfb0185929a56f1efd77992adbea06f781f504`.
`target/performance-workflow-comparison.json` retains both orders and all cases, whole
process counters, executable receipts and **1,049** after-source/config/harness hashes.
`performance-workflow-{before,after}-{forward,reverse}.json` contains raw samples;
the matching `*-run.json` files record commands, timestamps, successful exits and locks.
The complete pre-change inputs remain under `performance-workflow-before-source/`,
with their manifest. `performance-workflow-change-scope.json` records source invariants.
`performance-workflow-revalidation-runs.json` and
`performance-workflow-prior-evidence-revalidation.json` preserve the compatibility checks;
`performance-workflow-prior-evidence-ledger.json` preserves the prior ledger. The current
ledger pins 31 relevant named inputs per new variant; the broader comparison keeps the
complete discovered input guard. These pins do not cover every asset, external
toolchain or runtime condition. Gate details are under `performance-gate-pass19/` and
`performance-workflow-gate-receipt.json`.

Reproduce after building the optimized MCP binary, with other builders stopped:

```sh
python3 scripts/performance-mcp-workflows.py \
  --binary target/release/bruhs-mcp \
  --output target/performance-current-workflows.json
scripts/gate.sh --quick
```

Larger histories, long operations, granted/executed audit rows, output allocation/CPU,
slow clients and lock contention remain to measure. The full large response is still
expensive. Native record context/history, Unicode-heavy scans, providers, capture,
sync and physical UI actions remain in the active goal.

## Large MCP response storage reuse

The previous pass left a large process memory footprint after repeated complete audit
reads. This pass traces it through live process snapshots and removes repeated frame
and audit-text buffer allocation. The complete audit response remains unchanged.

### Observed memory growth and change

A fresh standalone process seeds 10,000 blocked Task dry runs, then reads the same
complete audit 35 times. Idle RSS snapshots and `vmmap -summary` show released large
allocation regions accumulating while the live stored history remains fixed. In the
baseline, RSS rises from about 15 MiB after seeding to **517 MiB** after 35 reads.
The final map reports roughly 494 MiB of resident `MALLOC_LARGE (empty)` regions, while
the default zone's live allocation count is unchanged from seeding. These observations
identify retained released allocation regions rather than additional workflow rows.

Reusing only the outer frame buffer reduces the growth but leaves roughly 287 MiB
after 35 reads. The final change also reuses the consumed audit text's storage; the
same fixture then stays near **33 MiB** after its first large read. The three processes
and their raw maps are retained as diagnostic stages. The inherited local allocator
environment includes `MallocNanoZone=0`; no allocator configuration is changed.

Idle **RSS MiB**, separate diagnostic processes rather than per-action peaks:

| Completed audit reads | Allocate both | Reuse frame only | Reuse both |
|---|---:|---:|---:|
| Seeded | 14.8 | 14.8 | 14.8 |
| 1 | 32.8 | 32.8 | 32.8 |
| 2 | 50.8 | 42.6 | 32.8 |
| 5 | 97.3 | 64.7 | 32.8 |
| 10 | 176.6 | 103.3 | 32.8 |
| 20 | 331.3 | 176.6 | 32.8 |
| 35 | 517.0 | 286.5 | 32.8 |

The stdio loop now owns one frame Vec and one spare text Vec. A response is completely
encoded into the cleared frame Vec, written with `write_all`, and flushed. Only after
that succeeds does the loop consume tool-result text and retain the single largest
allocation as empty scratch storage. A later standalone audit takes that storage and
pretty-serializes the complete current state into it. Other smaller responses do not
evict the larger spare. The canonical audit row/key serializers and the automation
lock scope are unchanged. No response value, grant decision or workflow result is
cached; every audit re-reads and encodes current state.

Both buffers retain capacity up to the largest encountered output until the serve
session ends. A single huge response can therefore keep large capacity alive. Stored
history, complete output text and the complete frame still scale with response size;
this is not constant whole-process memory or a new production byte limit.

Serialization errors clear all prior and partially serialized frame bytes before
encoding the existing internal-error response and newline. Write/flush errors still
end the loop. Response ordering, notifications, invalid/oversized input handling,
catalogs, lookup, argument validation, tool errors and native bridge calls remain
unchanged. Four new contracts check allocation reuse, large-to-small current audit
output, largest-spare retention and flush failure. The previous serialization-failure
contract now deliberately writes a large prefix before returning an error.

### Paired complete-process measurements

The accepted comparison uses fixed optimized MCP executables in before/after/after/
before order, with builders stopped and four Cargo profile locks held. It reuses the
previous fixture unchanged: four seed histories, all ten dry-run preview domains,
complete audits, rejected apply, invalid input, empty Chat/thread/task reads, ping and
the complete catalog. Each of the **18 cases per history** has five warmups and all
**30 raw samples per binary and order**. No measured sample is dropped. The combined
view pools 60 samples per binary; separate orders remain available.

All **144 paired case/order checks** preserve normalized result text, complete frame
sizes, envelopes and exact history ranges. Only validated UUIDs and known timestamp
fields are normalized. Full user text, Unicode, escaping, JSON layout, all array rows
and fields remain in the comparisons. Every write is a blocked synthetic dry run;
final state checks prove that no domain records, messages or jobs are created.

Maximum RSS for the **entire MCP process**, **MiB**:

| Seeded workflows | Forward: before | After | Reverse: before | After |
|---|---:|---:|---:|---:|
| 0 | 9.0 | 9.0 | 9.0 | 8.9 |
| 128 | 9.8 | 9.6 | 9.8 | 9.7 |
| 1,024 | 16.1 | 15.2 | 16.9 | 14.3 |
| 10,000 | 533.6 | 34.1 | 535.6 | 34.2 |

At 10,000 seed workflows, the two whole-process maximum RSS observations fall from
about **534–536 MiB to 34 MiB**, roughly 94% lower. These counters include seeding,
all cases and final audit checks; they are not per-audit allocation counts or memory
percentiles. Parent Python memory is excluded. The interactive combined view uses
the mean of two process counters, not a latency percentile. This repeated full fixture
is distinct from the idle snapshots above.

Complete audit stdio round trip, **p50 / p95 milliseconds**:

| Seeded workflows | Forward: before | After | Reverse: before | After |
|---|---:|---:|---:|---:|
| 0 | 0.088 / 0.104 | 0.102 / 0.133 | 0.089 / 0.107 | 0.094 / 0.112 |
| 128 | 0.447 / 0.511 | 0.441 / 0.475 | 0.463 / 0.689 | 0.454 / 0.536 |
| 1,024 | 2.970 / 3.425 | 2.776 / 3.466 | 2.913 / 3.340 | 2.814 / 3.476 |
| 10,000 | 30.875 / 31.841 | 29.434 / 30.158 | 31.353 / 32.272 | 29.690 / 30.424 |

The 10,000-workflow response remains **8,509,743 bytes**. Its median improves about
4.7% and 5.3% in the two orders. At 1,024 rows the median improves, while the p95 moves
slightly upward. Empty-state audit medians become roughly **5–14 microseconds slower**;
other small controls and tails remain mixed. All ten preview domains, errors and
read/control cases remain inspectable. This pass establishes a large retained-memory
improvement and modest large-audit latency improvement, not a universal speedup.

The timer starts at parent request write/drain and ends at the complete stdout
newline. Parent JSON parsing and validation are outside it. Cold first allocation,
startup/hydration, providers, concurrent clients, cancellation and physical input/
paint are excluded. Complete output still takes roughly 30 ms at this size.

### Encoding and backpressure checks

One optimized ignored fixture compares the former allocating `serde_json::to_vec`
plus newline with the current reusable frame encoder. It constructs the same response
outside the timer and verifies complete encoded bytes. The allocating variant includes
frame destruction; the reusable variant retains capacity between requests. Both run
orders retain five warmups and all 30 samples per case. The UTF-8/escaped text sizes
below produce different frame sizes from the actual audit, so these component times
must not be subtracted from the full-stdio times to infer transport overhead.

Encoding only, **p50 / p95 milliseconds**:

| Text size / encoded frame bytes | Forward: allocate | Reuse | Reverse: allocate | Reuse |
|---|---:|---:|---:|---:|
| 1,024 / 1,322 | 0.0009 / 0.0020 | 0.0007 / 0.0008 | 0.0009 / 0.0010 | 0.0007 / 0.0008 |
| 65,536 / 79,374 | 0.0437 / 0.1049 | 0.0942 / 0.0966 | 0.0891 / 0.0944 | 0.0799 / 0.0803 |
| 1,048,576 / 1,268,146 | 0.9040 / 1.2443 | 0.7034 / 0.8640 | 0.6582 / 0.7111 | 0.6462 / 0.7626 |
| 8,388,608 / 10,144,442 | 5.8471 / 6.4136 | 5.1403 / 5.5294 | 5.9238 / 7.5035 | 5.1883 / 5.6047 |

The roughly 10 MB frame improves in both orders. The 64 KiB case moves in opposite
directions, with a large forward shift; all raw samples remain. This component fixture
is a same-executable allocating-reference comparison, not a second full MCP binary
comparison or a guarantee for every response size.

`scripts/performance-mcp-backpressure.py` adds a real slow-reader functional check.
It uses an 8 KiB asyncio buffer and drains stdout in 4 KiB chunks with a 1 ms delay,
applying actual pipe backpressure. Five requests are pipelined: a complete 10,000-row
audit, ping, invalid write, valid blocked preview and a complete updated audit. Both
large frames require 2,078 reads. All five normalized response fingerprints and raw
frame sizes match before/after; the last audit includes the new workflow and an
additional ping confirms the session resumes. Intentional delays and seeding are
functional controls, not measured action latency or timeout acceptance thresholds.

### Verification and evidence

- The full quick gate passes formatting, strict native all-feature/all-target Clippy,
  WASM Clippy, **2,392 Rust tests with zero failures**, 25 manual ignores, **36 site
  tests**, site types and benchmark self-checks. All **77 MCP tests** pass with the
  single manual encoding fixture ignored; both optimized MCP/test builds succeed.
- Among **1,049 existing source/config/harness inputs**, only three production MCP
  files and two test files change. Shared models, permissions, grants, native readers,
  page/query helpers, schemas, protocol data types and save/apply/undo code are
  byte-identical. No dependency or production Rust module is added. The new Python
  backpressure harness brings the after comparison to **1,050 named inputs**.
- Four after-only compatibility runs preserve **180 prior full-stdio fingerprints
  and sizes** across generic/projected native readers, legacy Finance/discovery/error
  controls and original standalone/synthetic-bridge controls. The fixed native fixture
  executable predates the prior preview helper; native read sources are unchanged in
  this pass. Current shared-model lifecycle behavior passes in the rebuilt full gate.
  Historical native timings remain historical; these checks make no current-native
  optimization claim.
- The ledger now has **34 partial entries across 27 action/method identities**. Two
  entries add reused audit storage and frame encoding/backpressure evidence. The 31
  source-affected older entries are explicitly revalidated and source-repinned; the
  original unchanged Finance handler entry retains its prior evidence. This is not
  all-variants action acceptance or a completion percentage.

The before MCP SHA-256 is `e2b76d184c6b462a2b0f09f0eecfb0185929a56f1efd77992adbea06f781f504`;
the after MCP is `c63f72b657535e6cac1b22522d4829765521aad2e0c74efbdfa5c9bd7cf42f0d`.
The optimized encoding fixture is `9abefa3d48084d36094542aea0aa441403c4e2ea1dfa34f839853739c2541d81`.
`target/performance-output-comparison.json` retains all four locked run receipts,
raw-case summaries, complete-frame checks, whole-process counters and 1,050 after
source/config/harness hashes. Raw samples remain in
`performance-output-{before,after}-{forward,reverse}.json`; the matching `*-run.json`
records commands, executable/harness hashes, timestamps, successful exits and locks.
`performance-output-details.json` preserves component samples, all three idle-profile
stages and the slow-reader checks. Their raw maps match `performance-output-profile*-vmmap.log`. The frame-only
stage source and binary remain under `performance-output-frame-stage/` and
`performance-output-frame-only`.

`performance-output-before-source/` and its manifest preserve baseline inputs.
`performance-output-change-scope.json` verifies source invariants;
`performance-output-extra-runs.json`, `performance-output-revalidation-runs.json` and
`performance-output-prior-evidence-revalidation.json` preserve supporting checks.
The previous ledger remains in `performance-output-prior-evidence-ledger.json`.
Gate details are under `performance-gate-pass20/` and
`performance-output-gate-receipt.json`. New ledger entries pin 32 relevant named inputs;
the broader comparison retains the complete discovered input guard. Neither proves
every asset, external toolchain, allocator, platform or runtime condition.

Reproduce with builders stopped after optimized builds:

```sh
python3 scripts/performance-mcp-workflows.py \
  --binary target/release/bruhs-mcp --output target/performance-current-workflows.json
python3 scripts/performance-mcp-backpressure.py \
  --binary target/release/bruhs-mcp --output target/performance-current-backpressure.json
cargo test -p bruhs-mcp --release --locked large_response_frame_encoding_cost -- --ignored --nocapture
scripts/gate.sh --quick
```

Very large/long responses, jobs and granted/executed audit states, per-action allocation
profiles, cold first allocation, other platforms/allocators, client disconnects during
large frames, cancellation and concurrency remain separate variants. Output CPU and
transport throughput still need work. The wider UI, native context/history, provider,
capture and sync action catalogue remains in the active goal.

## Native context reads and transcript validation reuse

This pass extends the audit to seven exact-ID context variants: Meeting summary and
transcript, local note context, Person profile/observations/relationship items, and
completed Chat messages. The optimized baseline shows a roughly 3 ms foreground
transcript validation cost at 100,000 segments, including an exhausted page with no
returned records. Every request originally rebuilt its identity-validation map.

### Accepted change and integrity boundary

The native hub now owns its transcript collection through `TranscriptMap`, a thin
wrapper around the same `BTreeMap<String, Vec<TranscriptSegment>>`. It retains **one**
validation result, bound to its meeting key. An unchanged-map read of that meeting can
reuse the result. Every mutable borrow clears it before exposing the map or any rows;
mutable iteration uses the same guard. The underlying records are private and contain
owned plain data, so safe immutable access cannot mutate validation fields. Replacement,
rekeying, removal, insertion, row edits and unwinding after a mutable borrow all require
fresh validation. Cloning and deserialization start uncached. A mutation to any meeting's
transcript conservatively invalidates the one result, even when a particular edit makes
no data change.

The existing full-record validator still runs on the first read, after invalidation or
after switching to another cached meeting. It checks all rows, including corruption
beyond the requested page, with the same identity, duplicate, count, byte, finite-value
and timestamp rules. The shared validator and snapshot finalized-speech/replay/sorting
logic are unchanged in production. An experiment that stored smaller map values had
mixed latency results and was fully reverted; its four runs and source remain archived.

The MCP adapter changes only the validation call. It still checks current Read access
and input bounds before lookup, rejects ambiguous Meeting IDs, distinguishes unloaded
transcripts, and encodes current rows and current Meeting metadata on every response.
It does not cache output, grants or persistence receipts. The persisted `MeetingCache`
and its load, save, stream-recovery and receipt rules remain unchanged. The wrapper
serializes exactly as its original map; validation state is skipped.

### Fixtures and measurement boundary

The fixture uses **100,000 rows** in each paged history: transcript, completed Chat,
People observations and relationship items. Meeting/note/People source collections
have **100,001 identities**, with the requested identity last; Chat has **10,001 distinct
GPUI entities**. The note context returns one complete selected note, not a note-history
array. Profile context includes both 100,000-row counts while omitting their histories
from the profile record.

Rows include Greek, accents, dotted I, Kelvin sign, emoji, quotes, backslashes and
newlines. The final Chat message retains completed reasoning, model, timing, error and
tool-call fields; three interleaved live messages and private checkpoints stay excluded.
The final transcript segment retains its partial flag. Meeting context excludes its
synthetic local audio path, and People context excludes voice fingerprints. Existing
contracts separately preserve drafts/stashes, full context fields and page-byte limits.

Paged targets have first, deep, last, exhausted, invalid-offset and missing-ID cases;
singular targets have context and missing-ID cases: **30 cases per boundary/run**.
Five warmups precede all **30 measured samples per case**, with every measured sample
retained. Four fresh locked runs execute before/after/after/before with both fixed native
binaries already present and all builders stopped. The same optimized MCP transport is
fixed throughout. All **60 paired full-stdio case/order checks** preserve complete JSON
fingerprints and raw frame sizes. Direct handler assertions check row order/counts,
offsets, rich fields and exclusions. Final real-socket calls recheck revoked grants and
clean up the isolated connection. No domain records, jobs, provider calls or real user
workspaces are modified.

Handler timing starts at native handler entry and ends at returned JSON Value. Full
stdio timing starts at parent request write/drain and ends at the complete MCP stdout
newline, including the actual child and private Unix socket. Parent parsing/assertions
are outside it. Invalid offset arguments can be rejected by MCP before native dispatch.
These boundaries must not be subtracted to infer transport overhead.

The accepted gain is for a **warm unchanged transcript map**. Cold first validation,
reads after mutation, alternating meetings, cold hydration, provider work, connection
enablement, concurrent clients, cancellation and physical input/paint are excluded.
Cold/post-mutation reads still perform the complete validation scan. No cold latency,
whole-application latency or peak-memory improvement is claimed by this pass.

### Paired optimized results

Native handler, **p50 / p95 milliseconds**:

| Transcript case | Forward: before | After | Reverse: before | After |
|---|---:|---:|---:|---:|
| First 50 | 3.156 / 3.792 | 0.243 / 0.411 | 3.444 / 3.823 | 0.294 / 0.463 |
| Offset 99,949 / 50 | 3.274 / 3.640 | 0.275 / 0.330 | 3.217 / 3.831 | 0.253 / 0.324 |
| Offset 99,999 / 1 | 3.081 / 3.416 | 0.094 / 0.149 | 3.279 / 3.791 | 0.135 / 0.212 |
| Offset 100,000 / empty | 3.190 / 3.527 | 0.104 / 0.125 | 3.215 / 3.665 | 0.095 / 0.142 |

Complete MCP stdio, **p50 / p95 milliseconds**:

| Transcript case | Forward: before | After | Reverse: before | After |
|---|---:|---:|---:|---:|
| First 50 | 3.821 / 4.341 | 0.709 / 0.868 | 3.867 / 4.538 | 0.726 / 0.923 |
| Offset 99,949 / 50 | 3.800 / 4.294 | 0.684 / 0.814 | 3.769 / 4.504 | 0.710 / 1.007 |
| Offset 99,999 / 1 | 3.374 / 3.957 | 0.359 / 0.435 | 3.433 / 3.972 | 0.305 / 0.471 |
| Offset 100,000 / empty | 3.249 / 3.922 | 0.317 / 0.456 | 3.380 / 4.081 | 0.327 / 0.448 |

First/deep transcript page medians improve roughly 81–82% through full MCP stdio in
both orders. Last/exhausted pages also avoid rebuilding the full validation map.
They still resolve the current Meeting identity and encode the current result. The
first validation after a mutation remains linear; this table does not measure it.

Other context controls, complete MCP stdio, **p50 / p95 milliseconds**:

| Context / first page | Forward: before | After | Reverse: before | After |
|---|---:|---:|---:|---:|
| Meeting summary | 0.307 / 0.448 | 0.346 / 0.455 | 0.376 / 0.535 | 0.303 / 0.489 |
| Local note | 0.264 / 0.378 | 0.257 / 0.404 | 0.272 / 0.442 | 0.262 / 0.525 |
| Person profile | 0.801 / 1.060 | 0.804 / 1.154 | 0.827 / 0.975 | 0.844 / 1.038 |
| People observations | 0.962 / 1.135 | 0.971 / 1.147 | 1.018 / 1.409 | 1.012 / 1.167 |
| Relationship items | 1.023 / 1.290 | 1.057 / 1.221 | 1.128 / 1.343 | 1.168 / 1.361 |
| Completed Chat messages | 0.495 / 0.688 | 0.439 / 0.602 | 0.503 / 0.766 | 0.475 / 0.666 |

The other reader implementations are unchanged. Their small movements, slower People
controls and variable tails remain visible; no general context speedup is attributed
to them. Deep/final/exhausted pages, missing IDs, invalid offsets and p99 values remain
in the complete artifact and interactive comparison. Exact-ID scans and completed-
message filtering retain their original linear work.

### Verification and evidence

- The final quick gate passes formatting, strict native all-target/all-feature Clippy,
  WASM Clippy, **2,398 Rust tests with zero failures**, 26 manual ignores, **38 current
  site tests**, site types and benchmark self-checks. The bridge subset passes **120
  native contracts** with five manual fixtures ignored. Three additional map contracts
  cover mutation/repair, rekeying, key-bound results, cloning and serialization.
- A native regression warms validation, corrupts a duplicate ID beyond the first
  page, verifies rejection, repairs it through another mutation path, and rechecks
  grant revocation for existing and missing IDs. A shared-model regression exercises
  11 off-page corruptions and oversized record counts in both record and snapshot
  validation without changing their production code.
- The source guard checks **1,053 optimized-baseline inputs**, plus the added owned-map
  module for **1,054 after inputs**. The hub changes only its map type/import/default;
  the reader changes only validation dispatch. Shared-model production code is exactly
  baseline code, and the timing/data/check functions are identical apart from a separate
  appended regression. Original inputs before adding the fixture are preserved too.
- A small before/final-after fixture matches all **30 complete MCP payload fingerprints
  and sizes** with 123 rows; its timings are excluded. Four after-only prior-fixture
  runs also preserve **180 older protocol fingerprints/sizes** across generic,
  projected, Finance/error/discovery and standalone/synthetic-bridge cases. They use
  the freshly rebuilt native executable and fixed MCP transport. Historical timings
  remain historical.
- The ledger has **38 partial entries across 31 action/method identities**. Four new
  groups cover the seven context variants. The 20 source-affected older entries are
  explicitly revalidated and source-repinned. No whole action is promoted to all-
  variants acceptance or a completion percentage.

The before native SHA-256 is `ff834af98cb30d3d382ff01a5ecd8d4dafd565fbe0d411c9ac944aeff8580cb4`;
the accepted after native is `55a190824458b8a8349abe37e005bd954880936f68373b44db248f6874b22575`. The fixed MCP transport is
`c63f72b657535e6cac1b22522d4829765521aad2e0c74efbdfa5c9bd7cf42f0d`.
`target/performance-context-comparison.json` combines both orders and boundaries,
complete-stdio equality checks and the 1,054 named after-source/config/harness hashes.
`performance-context-{before,after}-{forward,reverse}.log` retains direct raw samples;
the seven corresponding `.VARIANT.json` files per run retain full-stdio samples,
fingerprints and frame sizes. Each `*-run.json` records commands, hashes, timestamps,
successful exits and profile locks. Build receipts, original/baseline source manifests,
`performance-context-change-scope.json`, small-fixture and prior-revalidation receipts
preserve provenance. Pins cover listed inputs, not every asset, toolchain or runtime.

The rejected smaller-map experiment remains under
`target/performance-context-map-experiment/`, including all four raw runs, its binary,
source and gate details. Its timings are not merged into the accepted comparison.
A transient builder preflight rejection launched no run; the later four-run series
completed normally. The final accepted series was collected fresh after both accepted
binaries existed.

An earlier debug gate link ran out of disk space. Only rebuildable
`target/debug/incremental` data was removed; all sources and measurement artifacts were
retained. The failed log and cleanup receipt remain, and the final full gate passes.
Workspace HEAD advanced during this pass, while named Rust/config/harness inputs stayed
under the source guard. The current site suite reports 38 tests; this pass changes no
site code. Gate details are under `performance-gate-pass21/` and
`performance-context-gate-receipt.json`.

Reproduce after optimized builds, with builders stopped:

```sh
BRUHS_PERF_MCP_BIN="$PWD/target/release/bruhs-mcp" \
BRUHS_PERF_CONTEXT_OUTPUT="$PWD/target/performance-current-context.json" \
target/release/deps/bruhs_desktop-eb5d5e922fc6f198 \
  context_native_read_action_cost --ignored --nocapture
scripts/gate.sh --quick
```

Use Cargo's printed native executable suffix if it differs. The native fixture supplies
the private connection and Python harness; leave `BRUHS_PERF_CONTEXT_SMOKE` unset for
the full corpus. Cold/post-mutation validation, alternating meetings, longer/byte-limited
rows, additional ID/history distributions, error/grant timing, capture/sync/provider
work and physical UI latency remain in the active goal.

## Saved transcript overlay validation and cache decoding

Saved native transcript validation previously cloned every durable fragment, cloned
its base transcript, cloned merge identity keys and sorted a merged transcript solely
to validate it. At 90,000 base segments, the untouched optimized diagnostic measured
about 26 ms for fragment validation and 76 ms for complete validated cache JSON decode.
These are isolated component costs, not whole application startup or navigation latency.

### Accepted implementation and integrity boundary

`StreamFragments.validate` retains its original page-layout guard, then passes borrowed
fragment references to the shared `validate_transcript_overlay`. The helper validates
all fragment identities first, applies the same complete-record finalized-speech
protection, constructs a merged view of borrowed rows and validates every merged row.
No transcript text or identity strings are cloned in this validation-only path. Sorting
is unnecessary for validation: record limits are still checked before iteration and all
malformed-row, duplicate, byte-limit and foreign-record failures retain the same error.
The actual publication merge still sorts exactly as before.

The shared private identity helper accepts standard-library exact-length reference
iterators. Existing record and snapshot APIs keep all field checks, size limits, error
messages, finalized-snapshot replay rules and sorting. The source guard proves this is
an iterator-plumbing change to their existing implementation. The new overlay API uses
the same validator. It does not memoize validation or trust cached counts.

The native publication merge, restoration, stream reducer, cache receipts, optimistic
annotations, deleted-row handling and recovery code are byte-identical to baseline.
The overlay retains the original ability to replace a malformed partial base row before
merged-view validation, while duplicates and malformed surviving base rows still fail.
The full cache decoder separately validates stored base transcripts before overlays,
just as before. Recovery's two validation steps remain: they inspect different states
around removals and must not be collapsed without preserving those refusal rules.

### Fixture and measurement boundary

A deterministic native fixture uses 128, 1,024, 10,000 and 90,000 base segments. Ten percent
are partial rows replaced by fragments; another ten percent are appended partial rows.
The largest overlay therefore contains 18,000 fragments and yields 99,000 merged rows.
Rows include Greek, accents, emoji, quotes and newlines plus 256 ASCII bytes. Its saved
cache is 51,802,099 bytes. The fixture's encoded bytes and SHA-256 match across all runs.

Each of four component boundaries has five warmups and 30 retained measured samples
at each size: 16 cases per run, 480 samples. Four fresh runs execute
before/after/after/before after both fixed optimized binaries exist. Builders are stopped
and native debug/release plus WASM debug/release Cargo profile locks are held. That
produces 1,920 raw samples and 32 paired case/order checks. The earlier diagnostic is
retained separately and excluded from the accepted four-run comparison.

- Validation timing covers actual `StreamFragments.validate` through its returned result.
- Decode timing covers `serde_json::from_slice` through a returned validated `MeetingCache`.
  Complete decoded-state equality is checked after the timer; decoded-cache destruction
  is also outside it.
- Restore timing covers actual `hub.restore_meetings` through completed GPUI update,
  including old-state replacement. The incoming shared cache clone is outside the timer;
  every restored row is checked afterward.
- Stream timing covers actual partial `MeetingEvent` application through completed
  GPUI update, including the fixture's per-row event clone. The intended row, row count
  and retained saved base are checked after timing. Restore and event implementations
  are unchanged controls.

Input construction and assertions are excluded. These fixtures perform no physical
input/paint, real workspace reads, disk writes, HTTP, microphone capture or provider work.
No whole startup, whole navigation or peak-memory improvement is claimed. Validation
still inspects the complete merged history. Actual restoration still owns/clones speech
and builds its index.

### Paired optimized results

Validation and validated cache JSON decoding, **p50 / p95 milliseconds**:

| Base rows / component | Forward: before | After | Reverse: before | After |
|---|---:|---:|---:|---:|
| 128 / validate | 0.026 / 0.030 | 0.009 / 0.011 | 0.025 / 0.030 | 0.009 / 0.015 |
| 128 / JSON decode | 0.090 / 0.097 | 0.076 / 0.079 | 0.090 / 0.117 | 0.073 / 0.082 |
| 1,024 / validate | 0.215 / 0.276 | 0.077 / 0.136 | 0.223 / 0.347 | 0.073 / 0.081 |
| 1,024 / JSON decode | 0.716 / 0.838 | 0.620 / 0.785 | 0.719 / 0.907 | 0.599 / 0.668 |
| 10,000 / validate | 2.300 / 2.756 | 0.796 / 0.952 | 2.362 / 2.908 | 0.729 / 0.958 |
| 10,000 / JSON decode | 7.735 / 8.229 | 6.264 / 6.720 | 7.719 / 8.374 | 6.180 / 6.652 |
| 90,000 / validate | 27.679 / 29.703 | 10.106 / 10.867 | 27.417 / 29.299 | 9.897 / 10.370 |
| 90,000 / JSON decode | 75.203 / 77.672 | 60.068 / 61.095 | 75.940 / 77.516 | 58.684 / 60.110 |

At 90,000 base rows, validation medians are 63.5% and 63.9% lower in the two orders;
validated decode medians are 20.1% and 22.7% lower. The complete-byte parser
and record checks remain. No speedup is attributed to unchanged controls.

Restore and stream-event controls, **p50 / p95 milliseconds**:

| Base rows / component | Forward: before | After | Reverse: before | After |
|---|---:|---:|---:|---:|
| 128 / restore | 0.026 / 0.032 | 0.026 / 0.027 | 0.025 / 0.033 | 0.025 / 0.030 |
| 128 / stream event | 0.001 / 0.002 | 0.001 / 0.002 | 0.001 / 0.001 | 0.001 / 0.001 |
| 1,024 / restore | 0.206 / 0.243 | 0.205 / 0.234 | 0.213 / 0.269 | 0.206 / 0.226 |
| 1,024 / stream event | 0.001 / 0.001 | 0.001 / 0.001 | 0.001 / 0.001 | 0.001 / 0.001 |
| 10,000 / restore | 2.604 / 3.144 | 2.382 / 2.858 | 2.486 / 2.974 | 2.406 / 2.802 |
| 10,000 / stream event | 0.001 / 0.003 | 0.001 / 0.002 | 0.001 / 0.001 | 0.001 / 0.001 |
| 90,000 / restore | 27.325 / 30.119 | 31.082 / 34.231 | 29.520 / 31.671 | 29.608 / 31.369 |
| 90,000 / stream event | 0.002 / 0.006 | 0.002 / 0.007 | 0.002 / 0.011 | 0.002 / 0.006 |

The 90,000-row restore control is about 14% slower after in the primary forward
series and roughly flat in reverse. A second complete fresh before/after/after/before
series repeats all 16 cases without source changes or replacing primary samples.
It confirms the validation/decode gains but also shows slower large restores in both
orders, including one elevated restore tail. The actual restore implementation is
byte-identical. Allocator/cache state and whole-load impact have not been isolated;
unchanged source alone does not establish that the complete load path is unaffected.

Follow-up at **90,000 base rows**, **p50 / p95 milliseconds**:

| Component | Forward: before | After | Reverse: before | After |
|---|---:|---:|---:|---:|
| Validation | 27.240 / 28.708 | 9.902 / 10.516 | 27.148 / 28.872 | 10.090 / 10.650 |
| JSON decode | 76.681 / 78.793 | 59.700 / 61.554 | 76.152 / 78.110 | 59.949 / 61.574 |
| Restore control | 27.475 / 29.901 | 31.648 / 53.513 | 28.910 / 31.531 | 31.342 / 33.307 |
| Stream-event control | 0.002 / 0.007 | 0.002 / 0.008 | 0.002 / 0.008 | 0.002 / 0.007 |

The follow-up adds 1,920 raw samples under `performance-overlay-followup-{before,after}-
{forward,reverse}.json` and corresponding logs; `performance-overlay-followup-comparison.json`
retains all summaries, identical-fixture checks, binary pins and the reason for repeating.
The interactive comparison exposes both series, both orders and all components. Small
stream-event shifts and p99 outliers also remain visible in the full data.

**Complete cache loading is not accepted by this pass.** Validation and JSON decode are
measurably faster; restore variation remains unresolved. Next, measure decode through
completed restore in one interval, then isolate allocator/state effects and the clone/
index work. Do not add separately collected medians to claim a whole-load improvement.

### Verification and provenance

- The full quick gate passes formatting, strict native all-target/all-feature Clippy,
  strict WASM Clippy, **2,403 Rust tests with zero failures**, 27 manual ignores,
  **38 site tests**, site types and benchmark self-checks.
- Four shared equivalence contracts compare the original owned-merge validator against
  the new borrowed implementation. They cover unchanged finalized rows, partial updates
  and appends, all eight finalized-field changes, 12 off-page fragment corruptions,
  surviving invalid base rows, duplicates, correction of an invalid partial base row,
  blank identities, 100,000-record limits and the combined 64 MiB text limit. Existing
  snapshot, replay, receipt, deletion and malformed-page contracts also pass.
- The regular native fixture preserves the complete serialized cache and restored
  transcript without resuming capture or claiming a complete search snapshot. Its
  state/data checks are unchanged between timed binaries.
- The baseline manifest pins 1,055 named Rust/config/MCP harness inputs. Only native
  validation dispatch and private shared-validator iterator plumbing change in existing
  sources. Adding the shared overlay module and its tests yields **1,057 after inputs**.
  Exact source transforms prove that existing validation checks and native publication,
  restore, stream and recovery implementations are unchanged. Pins cover listed inputs,
  not every asset, toolchain or runtime.
- An after-only native/private-socket compatibility run preserves all **30 earlier
  complete MCP context payload fingerprints and frame sizes** across seven context
  variants. Direct native and revoked-grant assertions also pass. The fixed MCP binary
  and response implementations are unchanged. Four source-affected context ledger
  entries are explicitly revalidated and repinned; their historical timings remain
  historical.
- The ledger now has **39 partial entries across 32 action/method identities**. The new
  MeetingDetail entry records supporting native cache-component evidence only. It does
  not assert a measured navigation latency or promote any whole action to acceptance.

The optimized before native SHA-256 is `c6127d0e4fa5275233a42433e7e419006ace7916aedacff267901c5b27059b58`;
the accepted after native is `a80c7fc4558d33e39ff90a0b086278802cdd21d7863ad21e04a8aab751b91bf1`.
`target/performance-overlay-comparison.json` retains both orders, summaries, paired
fixture checks and all 1,057 named after hashes. `performance-overlay-{before,after}-
{forward,reverse}.json` and corresponding logs retain all raw samples, fixture hashes,
encoded sizes, commands, binary/harness hashes, timestamps, successful exits and locks.
Build receipts, original/baseline/after manifests and source copies,
`performance-overlay-change-scope.json`, `performance-overlay-context-revalidation.json`
and `performance-overlay-prior-evidence-revalidation.json` preserve provenance.
`performance-overlay-gate-receipt.json` and `performance-gate-pass22/` hold the passing
gate. The copied gate differs only in its log directory.

The first strict check found one redundant clone in a new model test. Both owned build
process groups were terminated and observed stopped before correcting it; the first logs
and termination receipt remain under `performance-overlay-first-check/` and
`performance-overlay-first-check-aborted.json`. The corrected source has passing model
Clippy and a fresh full gate. Only inactive, rebuildable `target/debug/incremental`
compiler cache was removed to create verification space; sources, binaries and evidence
were retained. `performance-overlay-cache-cleanup.json` records the cleanup.

Reproduce after an optimized native test build, with builders stopped:

```sh
cargo test -p bruhs-desktop --bin bruhs-desktop --release --no-run --locked
target/release/deps/bruhs_desktop-eb5d5e922fc6f198 \
  native_overlay_action_cost --ignored --nocapture
scripts/gate.sh --quick
```

Use Cargo's printed executable suffix if it differs. Whole workspace cold loading,
backup reads/hash checks, foreground initialization/paint, more meeting distributions,
near-limit text timing, memory, capture/provider work and recovery HTTP latency remain
in the active goal. This pass reduces validation copies rather than measuring whole
startup or claiming every application action is now optimal.

## Native cache loading and borrowed transcript restoration

The preceding pass left slower large restore controls despite faster validation and
JSON decoding. This pass measures validated cache decoding through completed native
restoration in one continuous interval, with correlated stage timestamps. It removes
unnecessary restoration copies and merge identity work. The accepted controlled results
show consistent full-load gains for fragment-only histories and smaller overlap gains.
Mixed replacement is roughly flat; snapshot-only controls stay close. This is cache-
component evidence, not a whole application startup, navigation or input-to-paint claim.

### Production change and integrity boundary

`restore_meetings` now selects a meeting's fragment overlay while constructing its active
transcript. A borrowed lookup of fragment IDs chooses replacements before cloning final
rows. It avoids cloning base speech that will immediately be replaced and avoids a
transient map of owned base identity strings. Fragment traversal order and the original
owned-row sort by timestamp/ID are retained. When a validated cache has no base rows,
restoration clones and sorts its already unique fragment set directly; no identity merge
is needed. The original owned merge is retained only as a test oracle.

`MeetingCache` remains opaque outside its module. Deserialization still validates all
Meeting keys/headers, every base transcript, fragment pages, all fragment identities,
count/byte limits, malformed fields and finalized-speech restrictions. Its indexed live
update path validates incoming rows and updates an existing fragment position or appends
a new identity. The no-base fast path relies on those existing unique-fragment invariants;
it does not relax deserialization or skip checks on external inputs. The saved base and
fragment pages remain separate durable evidence. Restoration does not turn fragments
into a full server snapshot or resume recording.

Search completeness, metadata, snapshot/replay policies, deletions, stream reduction,
index contents, cache serialization and receipt rules are unchanged. Active rows remain
owned values; the saved cache and existing receipts keep their shared ownership. Snapshot-
only order stays unchanged; a fragment entry, including an empty one, retains the original
merged ordering and incomplete-search semantics. Histories with no loaded transcript stay
unloaded. A post-restore stream update still invalidates the proper fragment receipt
without changing the retained saved base.

### Why the final comparison shares a decoder

A five-second native diagnostic sample identifies transcript cloning, owned merging and
index construction in the original restore path. It also includes JSON parsing and other
fixture activity, so its counts are not CPU percentages or accepted latency comparisons.
The profiled diagnostic is retained separately from final timing evidence.

Two earlier candidates passed correctness and the full gate but failed broader timing
checks. A reference-vector selection/sort reduced restore cost while its separate binary
showed slower snapshot-only decoding and mixed whole-load results. A borrowed-map variant
with owned-row sorting restored snapshot controls but slowed fragment-only complete loads.
Their two binaries, source snapshots, gates and **16 timing runs** remain in separate
rejection archives. None of their samples is pooled into the accepted comparison.

The final controlled test compiles the complete frozen original restoration method
alongside current restoration in **one optimized executable**. Only the test reference's
name/visibility changes; a source guard verifies its entire body against the preserved
pre-change source. Both runtime-selected variants call one out-of-line `MeetingCache`
decoder. The decoder takes only the encoded bytes; it receives no variant flag. The
optimized symbol inventory confirms one decoder entry. The current variant calls the
actual production restoration method.

This removes a between-binary decoder-code difference from this comparison. It does not
prove why earlier decoder timings moved: allocation state and other runtime effects still
belong to the measured pipeline. In particular, mixed replacement's lower restore costs
do not establish a complete-load gain when other stages offset them. No independent
component medians are added to infer end-to-end latency.

### Fixtures and boundaries

The fixture has four sizes (128, 1,024, 10,000 and 90,000 rows), four distributions and two
target states: **32 cases per run**. Snapshot-only has no fragment entry; mixed overlays
replace ten percent and append ten percent; fragment-only has no stored base; all-overlap
replaces every partial base row. The largest mixed result has 99,000 active rows. Rows
contain Greek, accents, emoji, quotes/newlines and 256 ASCII bytes. Each timed fixture has
one meeting. Separate correctness checks cover multiple meetings and unsorted/tied rows.

Largest input distributions:

| Distribution | Base / fragment / active rows | Encoded cache bytes |
|---|---:|---:|
| Snapshot | 90,000 / 0 / 90,000 | 43,148,129 |
| Mixed | 90,000 / 18,000 / 99,000 | 51,802,099 |
| Fragments only | 0 / 90,000 / 90,000 | 43,428,537 |
| All overlap | 90,000 / 90,000 / 90,000 | 86,495,225 |

Empty-hub cases clear previous state outside the timer; encoded bytes and fixture data
are warm. Replacement cases include old hub state destruction and replacement. Each
sample starts before validated JSON decode and ends after completed GPUI restoration.
The intermediate timestamp gives correlated decode/restore stages for that same sample.
Input construction and complete decoded-cache/active-row assertions are outside timing.

Five warmups precede 30 retained samples per case. Four fresh runs execute
original/current/current/original with the fixed optimized executable, builders stopped
at preflight and native debug/release plus WASM debug/release Cargo profile locks held.
This retains **3,840 continuous samples**, corresponding stage samples and **64 paired
case/order fixture checks**. Every input fingerprint and encoded size matches. All
restored rows, cached fields, completeness and inactive-capture assertions pass.

The boundary excludes actual state-file/backup reads, file hashing, other workspace
caches and construction, OS-cold data, physical input/paint, HTTP, capture devices,
providers, cancellation/concurrent hydration and peak memory. The method is a native
cache component; the full startup/navigation action remains unaccepted.

### Controlled optimized results

Complete validated decode through completed restore, **p50 / p95 milliseconds**:

| 90,000-row distribution / target hub | Forward: original | Current | Reverse: original | Current |
|---|---:|---:|---:|---:|
| Snapshot / empty | 51.354 / 54.082 | 50.826 / 53.918 | 52.015 / 53.704 | 51.322 / 54.429 |
| Snapshot / replace | 58.216 / 61.081 | 59.137 / 62.642 | 59.110 / 61.707 | 59.321 / 62.987 |
| Mixed / empty | 83.974 / 86.256 | 79.413 / 82.594 | 81.814 / 88.167 | 80.120 / 83.412 |
| Mixed / replace | 89.663 / 96.995 | 89.264 / 92.023 | 88.845 / 93.736 | 89.453 / 93.602 |
| Fragments only / empty | 80.154 / 84.412 | 72.890 / 76.768 | 78.920 / 83.808 | 73.150 / 76.264 |
| Fragments only / replace | 92.770 / 96.588 | 81.478 / 84.408 | 89.461 / 93.296 | 82.170 / 85.034 |
| All overlap / empty | 131.268 / 135.035 | 126.406 / 131.831 | 132.842 / 141.606 | 130.117 / 135.244 |
| All overlap / replace | 143.897 / 149.760 | 139.842 / 144.275 | 145.139 / 153.968 | 141.020 / 145.871 |

Fragment-only complete-load medians improve in both orders for both target states.
Mixed empty-hub results have smaller gains, and mixed replacement is effectively flat.
All-overlap results improve modestly; snapshot-only replacement has small slower shifts.
The complete artifact retains all smaller cases and p99 tails. No universal latency or
whole-startup improvement is attributed to these selected distributions.

Restore stage within those same intervals, **p50 / p95 milliseconds**:

| 90,000-row distribution / target hub | Forward: original | Current | Reverse: original | Current |
|---|---:|---:|---:|---:|
| Snapshot / empty | 10.337 / 11.113 | 10.102 / 11.465 | 11.046 / 11.714 | 10.182 / 11.254 |
| Snapshot / replace | 18.048 / 19.738 | 18.251 / 19.543 | 18.303 / 19.421 | 18.254 / 19.463 |
| Mixed / empty | 23.933 / 26.152 | 19.964 / 21.320 | 22.142 / 27.512 | 20.467 / 21.709 |
| Mixed / replace | 32.189 / 36.161 | 28.960 / 30.755 | 29.433 / 34.643 | 29.474 / 31.352 |
| Fragments only / empty | 28.001 / 31.421 | 20.317 / 21.375 | 27.954 / 32.298 | 20.012 / 21.922 |
| Fragments only / replace | 39.366 / 42.738 | 29.353 / 32.436 | 36.831 / 41.715 | 29.437 / 31.256 |
| All overlap / empty | 33.886 / 36.483 | 29.261 / 32.203 | 35.471 / 40.364 | 31.158 / 33.376 |
| All overlap / replace | 47.021 / 50.411 | 42.369 / 44.656 | 48.667 / 55.193 | 42.431 / 45.079 |

The unchanged decoder still dominates large complete loads. Stage distributions are
correlated per iteration; their medians need not sum to the complete-load median.
Further decode/index/lifetime work needs its own measurements. Lower isolated restore
cost does not justify claiming a mixed replacement speedup that the total does not show.

### Fresh legacy component check

The original 16-case component fixture is also rerun in both orders against the fixed
pre-validation-change native binary and the final current binary. Its timing/data/state
assertion function bodies are unchanged. This check includes both the preceding validation
improvement and current restoration; it is not a fresh attribution for validation and is
not the continuous-load baseline above. All 32 paired case/order fixture checks match,
with **1,920 raw component samples** retained. Stream-event source is unchanged.

At 90,000 base rows, **p50 / p95 milliseconds**:

| Component | Forward: legacy | Current | Reverse: legacy | Current |
|---|---:|---:|---:|---:|
| Validation | 25.327 / 26.845 | 9.482 / 9.961 | 25.747 / 26.922 | 9.340 / 9.606 |
| JSON decode | 72.056 / 75.306 | 55.953 / 57.453 | 72.056 / 73.843 | 55.311 / 56.675 |
| Restore | 23.463 / 26.185 | 20.414 / 22.144 | 25.752 / 27.602 | 19.762 / 20.990 |
| Stream event | 0.002 / 0.008 | 0.002 / 0.006 | 0.002 / 0.006 | 0.002 / 0.004 |

This fresh restore component is faster than its legacy baseline in both orders. Earlier
phase22 slower controls remain historical observations, not replaced or reinterpreted as
current timings. Complete-load results above have a different boundary and baseline.

### Verification and provenance

- The final quick gate passes formatting, strict native all-target/all-feature Clippy,
  strict WASM Clippy, **2,406 Rust tests with zero failures**, 28 manual ignores,
  **38 site tests**, site types and benchmark self-checks.
- New contracts compare borrowed merging with the original owned algorithm across empty,
  unsorted, tied, overlapping and appended histories, including last-fragment-wins. A
  multiple-meeting contract checks exact ordering, all index positions/text-byte totals,
  empty fragments, fragment-only and unloaded histories, cached ownership, completeness,
  receipt invalidation and a subsequent stream update. The continuous fixture's regular
  contract preserves all four cache shapes at zero, one and 129 rows.
- Existing malformed saved-fragment/duplicate/finalized-rewrite tests still fail closed.
  Source guards prove validation, shared models, stream reduction, removals, replay,
  indexes, schemas, response handlers and receipt-page logic are unchanged. Only two
  production files change; the original merge/reference and shared decoder are test-only.
- The original snapshot pins 1,057 named Rust/config/MCP harness inputs, the initial timing
  baseline adds one fixture for 1,058, and final guards pin **1,060 inputs** including the
  new state/index contract and frozen-reference file. The original reference method is
  compared in full after renaming only. Listed pins do not cover every asset/toolchain.
- The ledger has **40 partial entries across 32 action/method identities**. One older
  source-affected cache entry is explicitly revalidated and repinned. Its phase22 timing
  scope remains historical. The new entry records the controlled interval and legacy
  component evidence; no whole action is promoted to all-variant acceptance.

The controlled optimized native SHA-256 is `546ed7860bdc2908227fe95e53f4ccb71485a3703bc1358e831d33dc468959ae`.
The fixed legacy native is `c6127d0e4fa5275233a42433e7e419006ace7916aedacff267901c5b27059b58`.
`target/performance-load-controlled-comparison.json` retains both orders, all stage
summaries, matching-fixture checks and the 1,060 after-source hashes. The four
`performance-load-controlled-{original,revised}-{forward,reverse}.json` files and logs
retain raw samples, explicit environment selection, commands, binary/harness/manifest
hashes, timestamps and profile locks. The four corresponding `performance-load-components-`
runs and `performance-load-components-comparison.json` hold the separate legacy check.

Build receipts, original/baseline/final manifests and source copies,
`performance-load-change-scope.json`, `performance-load-shared-decoder-symbol.json` and
`performance-load-fast-path-invariants.json` preserve the reference and implementation
boundary. `performance-load-gate-receipt.json` and `performance-gate-pass23/` preserve the
passing gate; its script differs only in the log directory. The prior-entry revalidation
receipt records exactly which source pins changed.

Rejected data, binaries, source and gate results remain under
`performance-load-reference-vector-experiment/` and `performance-load-owned-map-experiment/`,
each with a rejection receipt. The first diagnostic and native sampling command/output
are separate and excluded. Early fixture timestamp and type corrections remain in their
preflight logs. Only inactive rebuildable debug incremental compiler cache was removed
to create build space; source, binaries and all evidence were retained.

A source-only client-lifetime review also found that general/probe capture clients use
process slots, remote jobs clone a retained provider client, recovery holds its client
across feeds and SSE holds its client across reconnects. This finds no per-chunk/per-attempt
client construction to remove; it is not network-latency or connection-reuse acceptance.
The checked source hashes and limitations are in `performance-load-client-lifetime-review.json`.

Reproduce controlled runs after an optimized native test build, with builders stopped:

```sh
cargo test -p bruhs-desktop --bin bruhs-desktop --release --no-run --locked
BRUHS_PERF_LOAD_RESTORE=original \
target/release/deps/bruhs_desktop-eb5d5e922fc6f198 \
  native_complete_cache_load_cost --ignored --nocapture
BRUHS_PERF_LOAD_RESTORE=revised \
target/release/deps/bruhs_desktop-eb5d5e922fc6f198 \
  native_complete_cache_load_cost --ignored --nocapture
scripts/gate.sh --quick
```

Use Cargo's printed executable suffix if it differs. `BRUHS_PERF_LOAD_CASE` can isolate
`rows/distribution/target-state` for diagnostics; leave it unset for the full corpus.
Whole workspace loading, actual disk/backup/hash work, physical startup/paint, more meeting
and text distributions, error/cancellation/concurrency timing and peak memory remain in
the active all-action goal. Mixed replacement still needs improvement; this pass accepts
only the scoped cache implementation and controlled evidence above.

## Native workspace file loading and record transfer

Loaded workspace records now move into their runtime owners instead of being cloned
while the decoded copy waits to be dropped. In the final controlled corpus, warm
file-read-through-Root completion improves about **6–8%** for large knowledge files,
**11–12%** for large record collections, and **6–7%** for mixed workspaces across both
run orders and primary/backup routes. Constructor-only medians improve about 92% for
the large knowledge case, 69–70% for large records, and 50–52% for mixed records.
The primary reported interval includes reading, validation, version hashing or backup
recovery and construction; these are not claims about physical startup or paint.

The settings-only control is slower by roughly 10–49 microseconds at the file-to-Root
median across the named routes/orders, except one faster reverse backup run. The
forward backup control has a roughly 59% higher p95, 0.245 → 0.389 ms. Its full
headless-settled interval is also slower in the forward run. Small knowledge and
history/meeting controls are mixed. Those results remain visible; this pass does
not establish a universal startup speedup. JSON decoding still dominates the large
knowledge load, and saved chat messages still clone during thread restoration.

The production change is confined to `workspace.rs` and its `persistence.rs` helper:

- `Workspace::new` invokes the existing loader once, then a shared loaded-state
  constructor. Test-only restorer injection enables controlled original/current
  comparisons through that constructor; production directly invokes the transfer.
- `RestoredRecords::take_from_persisted` moves 14 owned record fields, including
  document bodies/history, notes, inbox/relationship records, life and automation
  state. Project/task/calendar/location DTO conversion moves its owned strings.
  Invalid calendar dates are filtered exactly as before, and the focus suggestion
  is seeded only when absent. Existing suggestions and their state remain intact.
- Every subsequent scalar preference read retains its original code. None of the
  transferred record fields is read again from saved state. Thread construction,
  meeting/people cache hydration, file loading/backup/hash checks, saving, snapshot
  generation, contact restoration and MCP handlers retain their previous bodies.

The ownership contract verifies buffer/string reuse, preservation of unrelated
saved fields, a non-default scheduling preference and an existing dismissed focus
suggestion. Full-constructor contracts compare complete snapshots against the
frozen original restorer for primary/recovery files, retain invalid-date behavior,
and cover a missing primary even when a backup exists. Fresh thread/default-rule
IDs are checked according to their generated-ID semantics. A fixture contract
checks reproducibility, and every measured window asserts rendered composer bounds,
the disk version/recovery state, zero save requests and no provider turn.

Seven synthetic shapes each use both primary and corrupt-primary backup routes:

- A settings control with one fixed empty thread; 256/4,096 knowledge documents,
  each with a full saved revision; and 4,096 each project/task/event/capture/note/
  clip/relationship/recent-project/finance records, with 256 inbox conversations.
- A chat control with 8,192 messages over 64 threads, a meeting control with 8,192
  transcript rows in one ended meeting, and mixed 1,024 documents/record collections,
  2,048 messages, 8,192 transcript rows and 256 inbox conversations.
- Complete Unicode, escaped quotes/newlines and record history are retained.
  Final input files range from a few kilobytes through roughly 43 MB. Original/current
  fingerprints and lengths match for every paired case and order.

The accepted comparison uses **one optimized executable**, one out-of-line observer
and one loaded-state constructor. Its frozen original record-restoration method
equals the pre-change source after renaming. Four fresh runs execute original /
transfer / transfer / original, with 5 warmups and 30 samples per case. All **1,680
continuous samples**, correlated read/decode/hash/construction/settle stages and
**28 paired case/order checks** pass. Full state checks are outside the timer;
correlated per-iteration intervals are combined before calculating percentiles.
The four native/WASM debug/release Cargo profile locks are held and builders are
absent at timing preflight. No profiler runs during accepted measurements.

The initial separate-binary 420-sample baseline is preserved with its source and
binary. Its fixture still generated default scheduling-rule UUIDs. That was corrected
before the final corpus was frozen; **no paired gain is calculated from that initial
binary**. Only the final same-binary runs with identical input fingerprints establish
the comparisons below.

Primary file; continuous **file read → Root ready**, milliseconds.

| Shape | Original first, p50 | Transfer first, p50 | p95 change, both orders |
|---|---:|---:|---:|
| Settings control | 0.222 → 0.253 (+13.8%) | 0.215 → 0.225 (+4.6%) | -5.7% / +3.0% |
| 256 knowledge documents | 3.958 → 3.952 (-0.2%) | 4.076 → 3.849 (-5.6%) | -3.1% / -6.0% |
| 4,096 knowledge documents | 62.668 → 58.928 (-6.0%) | 64.144 → 58.978 (-8.1%) | -15.7% / -5.4% |
| 4,096 records per collection | 34.263 → 30.126 (-12.1%) | 34.297 → 30.060 (-12.4%) | -11.6% / -11.8% |
| 8,192 messages | 16.934 → 16.669 (-1.6%) | 16.893 → 16.896 (+0.0%) | -1.5% / +0.7% |
| 8,192 transcript rows | 9.263 → 8.930 (-3.6%) | 8.953 → 9.031 (+0.9%) | -4.2% / -1.6% |
| Mixed workspace | 38.911 → 36.205 (-7.0%) | 38.612 → 36.374 (-5.8%) | -7.9% / -5.1% |

Corrupt primary → backup; continuous **file read → Root ready**, milliseconds.

| Shape | Original first, p50 | Transfer first, p50 | p95 change, both orders |
|---|---:|---:|---:|
| Settings control | 0.205 → 0.254 (+23.9%) | 0.228 → 0.213 (-6.5%) | +59.1% / -13.2% |
| 256 knowledge documents | 3.905 → 3.915 (+0.3%) | 4.155 → 3.912 (-5.9%) | -2.1% / -1.4% |
| 4,096 knowledge documents | 64.746 → 59.375 (-8.3%) | 64.064 → 59.640 (-6.9%) | -12.8% / -6.6% |
| 4,096 records per collection | 34.536 → 30.372 (-12.1%) | 34.242 → 30.515 (-10.9%) | -11.4% / -11.3% |
| 8,192 messages | 17.458 → 16.892 (-3.2%) | 17.031 → 16.916 (-0.7%) | +0.1% / +1.7% |
| 8,192 transcript rows | 9.373 → 9.001 (-4.0%) | 9.205 → 9.126 (-0.9%) | -4.4% / -0.8% |
| Mixed workspace | 38.957 → 36.574 (-6.1%) | 39.131 → 36.514 (-6.7%) | -6.7% / -7.3% |

The raw comparison also retains the continuous interval through initial **headless
GPUI settling**, and each individual stage. Large-data improvements remain in that
interval, while the small controls retain their mixed/slower results. Headless
executor completion and rendered composer bounds do not establish physical display
presentation or input-to-paint.

The final source is pinned across 1,062 named Rust/config/MCP harness inputs. Exact
scope checks prove the unchanged constructor initialization, persistence boundaries,
focus seed, frozen reference and 14-field lifetimes. These pins exclude unlisted
assets/toolchain inputs. The full quick gate passes: **2,409 Rust tests, 29 manual tests
ignored, 38 site tests**, native all-target/all-feature and WASM Clippy with `-D warnings`,
formatting, site types and benchmark self-checks. The gate subprocess passed before
its initial recorder encountered Node's Unicode summary format; the corrected receipt
records the completed logs without rerunning checks, and identifies its filesystem
time bounds explicitly.

Four after-only MCP compatibility runs cover generic, projected, finance and context
reads using the rebuilt native fixture and fixed accepted MCP transport. All **194
complete response fingerprints and byte counts** match the historical corpora.
The 18 prior entries whose file-level pins changed retain their original timings,
binaries and manifests, with explicit source/contract/protocol revalidation.
The ledger now contains **41 partial entries across 33 identities**; initial native
Chat loading has been mapped without promoting any whole action to accepted.

Evidence and reproduction:

- `target/performance-workspace-load-comparison.json` and the interactive
  `target/performance-workspace-load-comparison.html` retain both orders, routes,
  medians and tails. The four corresponding `after-{original,current}-{forward,reverse}`
  JSON/log pairs retain raw samples, fingerprints, commands, explicit environment
  selection, binary/harness/manifest hashes, timestamps and profile locks.
- Original/baseline/final manifests and source copies, build receipts,
  `performance-workspace-load-change-scope.json`, constructor extraction and shared
  symbol proofs, regular contract/Clippy logs, gate logs/receipt, compatibility
  files and the prior-evidence revalidation receipt remain under `target/`.
- The final binary is `target/performance-workspace-load-after`, SHA-256
  `c1f5c5ba8a01b97ab319a2192b7097fa2b93ba330980cbda9e4c3dd47988e3a9`. Run
  `python3 target/performance-workspace-load-run.py after original forward`, then
  `after current forward`, `after current reverse`, `after original reverse`, and
  `python3 target/performance-workspace-load-compare.py`. The runner verifies source
  pins and the binary and refuses to start with active builders/occupied profile locks.
  `python3 target/performance-workspace-load-revalidate.py` repeats compatibility.

Physical startup/paint, OS-cold storage, network/update hydration, capture/providers,
peak memory, live queued work/concurrent changes, more record/text distributions
and malformed/both-backup error timing remain unmeasured. Warm file decode is still
synchronous. This is a bounded native loading improvement; the broad goal stays active.

## Native saved-thread restoration and complete file loading

Saved threads transfer message and session data into runtime ownership. The alternating
user/assistant conversation (4,096 messages over 32 threads) shows **12–15% faster from
file read through Root completion**, and **10–12% faster through headless settling**. Its
constructor is about 82–84% faster. The separate user-role copy-stress history shows
15–17% faster to Root completion. The previous record-transfer optimization stays enabled
in both modes.

Complete intervals, controls and tails remain explicit. The largest headless p95 increase
in this corpus is 1 thread × 32 user-role copy-stress rows / backup in the forward order:
4.089 → 4.454 ms (+8.9%). No case is promoted to whole-action startup or tail acceptance;
component gains do not establish gains for every complete interval.

The change is limited to three production files:

- The workspace computes the original model and permission state first, then
  hands mutable loaded state to thread restoration at the same initialization
  point. No later constructor code reads the saved thread vector. Test-only
  strategy injection lets both implementations share that constructor.
- Thread restoration takes the saved vector and consumes each thread. It moves
  message content/thinking, nested tool input/output, file name/path metadata,
  errors/checkpoints, import context, goals, worktree/effort/role/stamp state,
  queued prompts/citations and provider session ids. The current model is borrowed.
  Strings retained by two runtime owners still receive their required copy.
- The owned-session helper retains the existing valid-provider/nonempty-id policy.
  The old borrowed session/model helpers remain test-only for original/reference
  and compatibility checks. Saved schemas, model/permission fallback, ordering,
  active-thread selection, legacy activity times and queue ownership filtering
  retain their behavior. The file reader, backup/hash/writer, snapshot generator,
  earlier record-transfer helper, chat/queue behavior and action handlers are unchanged.

The existing workflow test now passes an explicit owned clone because it reuses its
checkpoint; its assertions remain unchanged. New contracts compare complete loaded
snapshots for valid/invalid/empty/unowned sessions, a blank model/path, active-thread
permission fallback and recovery. Fresh/missing/empty states and legacy timestamps
match the frozen implementation after checking their intended generated ids/times.
Pointer checks prove reuse of message/thinking strings, nested JSON/tool/file storage,
import context, session ids and queued text, with all unrelated saved fields intact.
Queued prompts remain held, foreign-thread prompts are removed, and every timed
window asserts rendered composer bounds, zero save requests and no provider turn.

The corpus retains all seven prior shapes and adds five:

- One thread with 32 user-role copy-stress rows; 32 threads with 128 such rows each;
  one thread with 8,192 plain messages; 512 threads with two messages each; and
  a separate 32-thread × 128-message alternating user/assistant conversation.
- User-role copy-stress rows retain thinking, usage and timing, stopped/error state, empty/valid
  checkpoints, two nested JSON tool calls and four uploaded-file name/path records.
  The alternating case places files on user rows and thinking/tools/usage on assistant
  rows, with Markdown content and no uploaded files on assistant rows. Threads retain
  provider sessions, import text, paused goals, worktree/effort/role
  metadata and held queues with citations. Actual uploaded file bytes/I/O are excluded.
- Each of the twelve shapes uses both the primary file and a deliberately corrupt
  primary with a valid backup: **24 cases**, with complete UTF-8 and escaped content.

Four fresh runs use original / owned / owned / original in **one optimized executable**,
with one out-of-line observer and one loaded-state constructor. The two frozen
original thread method bodies equal pre-change source after renaming. Each case has
five warmups and 30 accepted samples: **2,880 continuous observations and 48 paired
case/order checks**, with correlated load/construction/headless-settle timestamps.
Complete snapshots and file bytes are checked outside timing. The runner clears
the diagnostic case filter, checks source/binary pins, requires no builders, and
holds all four native/WASM debug/release Cargo profile locks. Run-order labels refer
to variant order; the corpus order stays fixed. No accepted run is profiled.

A fresh unchanged prior `c1f5c5…e3a9` executable supplies a separate 420-sample baseline.
All 14 original input fingerprints/lengths match the final corpus. Those timing
samples are retained as a diagnostic baseline and are not pooled into paired gains.
The final original/owned comparison determines every reported gain below.
An initial expanded forward pair was also preserved and excluded after the next
preflight detected a transient Cargo process. It had exited before inspection, so
overlap could not be reconstructed. The final figures use a fresh uninterrupted
four-run cohort; its source pins, inputs and binaries remain fixed. The excluded
pair and inspection are in `target/performance-thread-preflight-interrupted/`.
The earlier 22-case corpus used user-role rows for its rich payload stress. It was
preserved, including its source, binary, raw data, gate logs and draft report, at
`target/performance-thread-user-role-corpus/`. Production code did not change when
assistant coverage was added. All final numbers come from the new 24-case runs;
all 22 archived input fingerprints are also checked against the expanded corpus.

Primary file; continuous **file read → Root ready**, milliseconds.

| Shape | Original first, p50 | Owned first, p50 | p95 change, both orders |
|---|---:|---:|---:|
| Settings control | 0.257 → 0.258 (+0.2%) | 0.215 → 0.227 (+5.7%) | -9.0% / +19.5% |
| 256 knowledge documents | 3.942 → 4.030 (+2.2%) | 3.986 → 4.088 (+2.6%) | +3.8% / +0.9% |
| 4,096 knowledge documents | 61.051 → 60.837 (-0.4%) | 60.869 → 60.730 (-0.2%) | +2.5% / +0.0% |
| 4,096 records per collection | 30.890 → 30.836 (-0.2%) | 30.871 → 30.878 (+0.0%) | -1.2% / +3.5% |
| 64 threads × 128 messages | 16.956 → 16.039 (-5.4%) | 17.053 → 16.060 (-5.8%) | -6.1% / -6.9% |
| 8,192 transcript rows | 8.969 → 9.094 (+1.4%) | 9.192 → 9.036 (-1.7%) | -1.4% / -2.4% |
| Mixed workspace | 37.268 → 36.937 (-0.9%) | 37.328 → 36.696 (-1.7%) | -2.5% / -1.8% |
| 1 thread × 32 user-role copy-stress rows | 0.735 → 0.718 (-2.3%) | 0.723 → 0.659 (-8.8%) | +5.4% / -2.1% |
| 32 threads × 128 user-role copy-stress rows | 63.687 → 54.365 (-14.6%) | 65.074 → 54.104 (-16.9%) | -15.7% / -17.0% |
| 32 threads × 128 alternating user/assistant rows | 37.287 → 32.601 (-12.6%) | 38.085 → 32.476 (-14.7%) | -11.4% / -13.8% |
| 1 thread × 8,192 messages | 15.651 → 14.882 (-4.9%) | 15.873 → 14.837 (-6.5%) | -6.1% / -7.0% |
| 512 threads × 2 messages | 12.633 → 12.816 (+1.4%) | 12.985 → 12.620 (-2.8%) | +0.5% / -2.0% |

Corrupt primary → backup; continuous **file read → Root ready**, milliseconds.

| Shape | Original first, p50 | Owned first, p50 | p95 change, both orders |
|---|---:|---:|---:|
| Settings control | 0.243 → 0.240 (-1.3%) | 0.233 → 0.238 (+2.0%) | +17.9% / -12.9% |
| 256 knowledge documents | 4.038 → 4.078 (+1.0%) | 4.034 → 4.021 (-0.3%) | -2.7% / +1.3% |
| 4,096 knowledge documents | 60.934 → 60.882 (-0.1%) | 60.900 → 61.036 (+0.2%) | -0.9% / +0.9% |
| 4,096 records per collection | 30.979 → 31.079 (+0.3%) | 31.054 → 31.161 (+0.3%) | -0.9% / +0.8% |
| 64 threads × 128 messages | 17.487 → 16.180 (-7.5%) | 17.242 → 16.075 (-6.8%) | -5.3% / -7.1% |
| 8,192 transcript rows | 9.016 → 9.135 (+1.3%) | 9.265 → 9.134 (-1.4%) | -1.3% / +0.7% |
| Mixed workspace | 37.493 → 37.084 (-1.1%) | 37.255 → 36.867 (-1.0%) | -3.1% / -0.9% |
| 1 thread × 32 user-role copy-stress rows | 0.707 → 0.680 (-3.9%) | 0.772 → 0.685 (-11.2%) | +5.1% / -6.4% |
| 32 threads × 128 user-role copy-stress rows | 64.542 → 54.723 (-15.2%) | 65.156 → 54.666 (-16.1%) | -16.4% / -15.9% |
| 32 threads × 128 alternating user/assistant rows | 37.475 → 32.859 (-12.3%) | 38.075 → 32.394 (-14.9%) | -12.0% / -13.1% |
| 1 thread × 8,192 messages | 15.652 → 14.860 (-5.1%) | 15.810 → 14.850 (-6.1%) | -6.4% / -8.1% |
| 512 threads × 2 messages | 13.267 → 12.868 (-3.0%) | 13.173 → 12.866 (-2.3%) | -8.0% / -3.3% |

The raw comparison and interactive view also retain the continuous interval through
headless settling and each individual stage. The paired percentile results remain
visible in both orders/routes, including all slow/mixed tails. Constructor
component gains do not substitute for complete-loading gains in the mixed/control cases.

Three separate sampled runs, after timing and compatibility checks, guide the next
loop. The knowledge case includes JSON string parsing/unescaping under the actual
saved-file reader. The 512-thread case includes sidebar, text shaping and Taffy/GPUI
layout stacks; the alternating-conversation sample retains Markdown, text-shaping
and layout paths for inspection. The 512-thread case spends **35.3–35.9 ms settling the headless initial
frame**, beyond 10.4–10.6 ms of construction. These are next targets, not physical
display measurements. The full diagnostic fixture also snapshots/asserts/tears down;
whole-process sample percentages cannot establish stage latency. All six profiled
case rows are excluded from the accepted comparison.

The full quick gate passes: **2,414 Rust tests, 30 manual tests ignored, 38 site tests**,
native all-target/all-feature and WASM Clippy with `-D warnings`, formatting, site
types and benchmark self-checks. Four after-only MCP runs recheck generic, projected,
finance and context reads; **194 complete response fingerprints and byte counts**
match their historical corpora. The 19 source-affected prior entries retain their
original binaries/timings/manifests with explicit source/contract/protocol revalidation.
The ledger now has **42 partial entries across 33 identities**. Complete Chat
interaction and the all-action objective remain unaccepted.

Evidence and reproduction:

- `target/performance-thread-load-comparison.json` and
  `target/performance-thread-load-comparison.html` retain all routes, orders,
  percentiles and boundaries. Four `{original,owned}-{forward,reverse}` JSON/log
  pairs retain raw samples, byte fingerprints, commands, explicit environment,
  binary/harness/manifest hashes, timestamps and Cargo locks.
- Original (1,062 inputs), fixture/final (1,064 inputs) manifests and source copies,
  the original baseline and final build receipts, scope/reference and shared-symbol
  proofs, contracts/Clippy/smoke logs, quick-gate receipt/logs, compatibility files,
  separate diagnostics and prior-evidence revalidation remain under `target/`.
  The pins cover named Rust/config/MCP inputs, not every asset or toolchain input.
- The final executable is `target/performance-thread-load-after`, SHA-256
  `955587fe52c6df6177eeb910e8f727b9d98a2e08eb4f235831af2ce1a25dff1a`. Run
  `python3 target/performance-thread-load-four-runs.py` for the guarded paired corpus,
  or `python3 target/performance-thread-load-run.py original forward` followed by
  `owned forward`, `owned reverse`, `original reverse`, then
  `python3 target/performance-thread-load-compare.py`.
  `python3 target/performance-thread-load-revalidate.py` repeats MCP compatibility;
  `python3 target/performance-thread-load-diagnostic.py` runs excluded sampled diagnostics.

The broad goal remains active. JSON read/decode/hash, initial rendering with many
threads, other action/error/concurrency shapes, OS-cold storage, actual font/app
initialization, physical startup/input-to-paint, live network/capture/provider work,
reading/writing attachment bytes and peak memory still need evidence and further work.

## Native saved-file JSON decoding

Saved-file loading validates UTF-8 once with the safe `simdutf8` API, then uses the
existing JSON decoder's string-input path. The 4,096-document case is 13.2–13.8% faster
from file read through Root completion and 12.1–13.3% faster through headless settling.
The alternating conversation shows 14.4–15.6% and 12.7–13.3% faster complete intervals
respectively. Both variants keep owned records and threads.

The production change replaces only the saved-state decode call and adds one small
helper plus `simdutf8` 0.1.5 as a desktop dependency. No existing dependency version
changes. The file reader still rejects non-files, reads the same bytes, hashes only a
successful decode, returns the same disk version, and follows the same backup/recovery
policy. Saved schemas, ownership transfer, constructors, queues, actions and writers
retain their code.

The [validator's safe API](https://docs.rs/simdutf8/0.1.5/simdutf8/basic/fn.from_utf8.html)
returns validated borrowed text. The pinned `serde_json` 1.0.149 implementation retains
JSON escape/control checks in `StrRead` while avoiding repeated UTF-8 checks on each
decoded string. If byte validation fails, the helper invokes the original slice
parser. That fallback retains exact serde diagnostics and legacy acceptance of invalid
UTF-8 inside ignored unknown values; it does not normalize or replace saved bytes.
Installed dependency and JSON-reader source were inspected and pinned in the evidence.

Four new regular contracts cover full rich state, deterministic input bytes, Unicode
and SIMD block boundaries, raw/escaped/control text, invalid sequences, duplicate
fields, recursion behavior, unknown enum values, exact error text/category/line/column,
raw-byte disk versions, missing/invalid/directory primary paths, valid recovery, and
both-invalid recovery. Existing state-file conflict, durability, locking and permission
tests also pass in the full gate. Full timed observations retain complete snapshot,
held-queue, zero-save/no-provider, rendered-composer, recovery/version and file-byte
assertions after timing.

An isolated reduced-schema probe distinguished standard UTF-8 prevalidation from SIMD
prevalidation. It is exploratory, not application evidence. The final decoder-only
experiment uses the actual `PersistedWorkspaceState` schema: 18 shapes, slice / standard
validation + string / SIMD + string, then reversed mode order. Its **3,240 separate
decode-only samples** retain decoded-state equality. They exclude I/O, hashing, runtime
construction, frames, assertions and destruction, and are not pooled into full-loading
gains.

The accepted comparison uses the actual production reader and a frozen copy of the
three original reader/backup helpers, equal to pre-change source after renaming and
blank-line formatting. Both paths call one shared out-of-line observer and the same
actual `Workspace::from_loaded_state` and Root constructor in one optimized executable.
The existing observer body changes only its injected loader call. All **24 prior input
fingerprints and byte counts match**. Six additional ASCII/Unicode/escaped compact and
pretty shapes retain real saved-state records and messages: **18 shapes × primary or
corrupt-primary backup = 36 cases**.

Four fresh runs use original / SIMD / SIMD / original, five warmups and 30 accepted
observations per case: **4,320 continuous observations and 72 paired case/order checks**.
The runner verifies sources/binary/harness, clears optional fixture selectors, requires
no active builders at preflight, and holds all four native/WASM debug/release Cargo
profile locks. No accepted run is profiled. Each iteration retains correlated read/
decode/hash, construction, headless settling and total timestamps. File-read-through-
Root percentiles are calculated from each iteration's complete interval; independent
stage medians are not added. Order labels describe variant ordering, not reversed
corpus order.

Continuous **file read → Root ready**, milliseconds. Negative changes are faster.

| Shape / route | Original first, p50 | SIMD first, p50 | p95 change, both orders |
|---|---:|---:|---:|
| Settings control / primary | 0.256 → 0.301 (+17.4%) | 0.230 → 0.222 (-3.3%) | +38.5% / -7.4% |
| Settings control / backup | 0.260 → 0.287 (+10.2%) | 0.231 → 0.237 (+2.4%) | +16.5% / +8.1% |
| 256 knowledge documents / primary | 4.055 → 3.534 (-12.9%) | 3.941 → 3.402 (-13.7%) | -14.3% / -12.3% |
| 256 knowledge documents / backup | 4.051 → 3.558 (-12.2%) | 3.990 → 3.431 (-14.0%) | -12.2% / -13.0% |
| 4,096 knowledge documents / primary | 60.728 → 52.732 (-13.2%) | 60.772 → 52.414 (-13.8%) | -12.8% / -15.7% |
| 4,096 knowledge documents / backup | 60.787 → 52.765 (-13.2%) | 60.660 → 52.373 (-13.7%) | -12.0% / -13.3% |
| 4,096 records per collection / primary | 30.911 → 27.730 (-10.3%) | 30.893 → 27.706 (-10.3%) | -10.8% / -9.8% |
| 4,096 records per collection / backup | 31.175 → 27.777 (-10.9%) | 30.966 → 27.782 (-10.3%) | -11.8% / -9.7% |
| 64 threads × 128 messages / primary | 16.114 → 14.020 (-13.0%) | 16.066 → 14.069 (-12.4%) | -11.7% / -10.9% |
| 64 threads × 128 messages / backup | 16.002 → 14.110 (-11.8%) | 16.200 → 14.070 (-13.1%) | -9.7% / -12.1% |
| 8,192 transcript rows / primary | 9.140 → 7.867 (-13.9%) | 9.111 → 7.899 (-13.3%) | -16.3% / -13.4% |
| 8,192 transcript rows / backup | 9.308 → 8.014 (-13.9%) | 9.226 → 7.933 (-14.0%) | -12.7% / -13.1% |
| Mixed workspace / primary | 37.077 → 32.090 (-13.5%) | 36.869 → 31.964 (-13.3%) | -13.7% / -12.4% |
| Mixed workspace / backup | 37.372 → 32.099 (-14.1%) | 36.969 → 31.908 (-13.7%) | -23.9% / -12.8% |
| 32 user-role copy-stress rows / primary | 0.715 → 0.570 (-20.3%) | 0.681 → 0.614 (-9.9%) | -13.0% / -14.6% |
| 32 user-role copy-stress rows / backup | 0.723 → 0.612 (-15.4%) | 0.749 → 0.591 (-21.1%) | -3.3% / -14.9% |
| 4,096 user-role copy-stress rows / primary | 54.260 → 46.369 (-14.5%) | 54.612 → 46.436 (-15.0%) | -14.8% / -14.6% |
| 4,096 user-role copy-stress rows / backup | 54.969 → 46.729 (-15.0%) | 55.070 → 46.658 (-15.3%) | -15.2% / -15.1% |
| 32 × 128 alternating user/assistant rows / primary | 32.625 → 27.708 (-15.1%) | 32.711 → 27.895 (-14.7%) | -13.4% / -15.3% |
| 32 × 128 alternating user/assistant rows / backup | 32.980 → 27.830 (-15.6%) | 32.864 → 28.147 (-14.4%) | -16.6% / -7.5% |
| One thread × 8,192 messages / primary | 14.914 → 12.600 (-15.5%) | 14.910 → 12.653 (-15.1%) | -14.3% / -14.4% |
| One thread × 8,192 messages / backup | 14.963 → 12.623 (-15.6%) | 14.799 → 12.580 (-15.0%) | -16.3% / -13.6% |
| 512 threads × 2 messages / primary | 12.703 → 12.425 (-2.2%) | 12.709 → 12.229 (-3.8%) | -6.3% / -6.3% |
| 512 threads × 2 messages / backup | 12.891 → 12.678 (-1.7%) | 12.988 → 12.543 (-3.4%) | -3.7% / -5.6% |
| ASCII / compact / primary | 3.581 → 3.292 (-8.1%) | 3.346 → 3.261 (-2.6%) | -5.1% / -8.3% |
| ASCII / compact / backup | 3.621 → 3.378 (-6.7%) | 3.595 → 3.347 (-6.9%) | -4.0% / -9.4% |
| ASCII / pretty / primary | 3.776 → 3.573 (-5.4%) | 3.722 → 3.484 (-6.4%) | -7.9% / -10.1% |
| ASCII / pretty / backup | 3.830 → 3.575 (-6.7%) | 3.633 → 3.465 (-4.6%) | -8.9% / -10.4% |
| Unicode / compact / primary | 7.470 → 4.706 (-37.0%) | 7.210 → 4.771 (-33.8%) | -35.7% / -34.4% |
| Unicode / compact / backup | 7.640 → 4.787 (-37.3%) | 7.376 → 4.766 (-35.4%) | -34.1% / -36.3% |
| Unicode / pretty / primary | 7.844 → 5.098 (-35.0%) | 7.643 → 4.978 (-34.9%) | -36.2% / -33.5% |
| Unicode / pretty / backup | 7.763 → 5.112 (-34.1%) | 7.618 → 5.054 (-33.7%) | -35.1% / -33.3% |
| Escaped text / compact / primary | 8.433 → 7.107 (-15.7%) | 8.098 → 6.935 (-14.4%) | -6.8% / -14.1% |
| Escaped text / compact / backup | 8.449 → 7.021 (-16.9%) | 8.220 → 6.987 (-15.0%) | -16.1% / -17.2% |
| Escaped text / pretty / primary | 8.652 → 7.361 (-14.9%) | 8.489 → 7.142 (-15.9%) | -16.7% / -13.3% |
| Escaped text / pretty / backup | 8.745 → 7.341 (-16.1%) | 8.532 → 7.181 (-15.8%) | -19.4% / -16.4% |

Continuous **file read → headless settling**, milliseconds.

| Shape / route | Original first, p50 | SIMD first, p50 | p95 change, both orders |
|---|---:|---:|---:|
| Settings control / primary | 2.611 → 2.794 (+7.0%) | 2.541 → 2.553 (+0.5%) | +10.7% / -1.2% |
| Settings control / backup | 2.605 → 2.780 (+6.7%) | 2.501 → 2.549 (+1.9%) | +8.0% / -0.2% |
| 256 knowledge documents / primary | 6.544 → 6.038 (-7.7%) | 6.435 → 5.910 (-8.2%) | -9.4% / -5.2% |
| 256 knowledge documents / backup | 6.589 → 6.218 (-5.6%) | 6.466 → 5.973 (-7.6%) | -5.7% / -7.6% |
| 4,096 knowledge documents / primary | 64.324 → 56.553 (-12.1%) | 64.452 → 55.906 (-13.3%) | -12.3% / -14.6% |
| 4,096 knowledge documents / backup | 64.413 → 56.486 (-12.3%) | 64.419 → 55.901 (-13.2%) | -10.3% / -12.8% |
| 4,096 records per collection / primary | 35.684 → 32.592 (-8.7%) | 35.806 → 32.629 (-8.9%) | -8.8% / -7.8% |
| 4,096 records per collection / backup | 35.984 → 32.746 (-9.0%) | 36.047 → 32.749 (-9.1%) | -9.6% / -8.7% |
| 64 threads × 128 messages / primary | 20.473 → 18.429 (-10.0%) | 20.507 → 18.406 (-10.2%) | -8.7% / -10.1% |
| 64 threads × 128 messages / backup | 20.567 → 18.632 (-9.4%) | 20.729 → 18.719 (-9.7%) | -8.4% / -9.2% |
| 8,192 transcript rows / primary | 12.723 → 11.423 (-10.2%) | 12.674 → 11.398 (-10.1%) | -13.0% / -10.7% |
| 8,192 transcript rows / backup | 12.910 → 11.611 (-10.1%) | 12.804 → 11.590 (-9.5%) | +23.2% / -7.5% |
| Mixed workspace / primary | 42.242 → 37.115 (-12.1%) | 41.840 → 36.851 (-11.9%) | -12.7% / -11.2% |
| Mixed workspace / backup | 42.548 → 37.122 (-12.8%) | 42.193 → 37.013 (-12.3%) | -21.0% / -11.2% |
| 32 user-role copy-stress rows / primary | 3.890 → 3.537 (-9.1%) | 3.745 → 3.693 (-1.4%) | -3.4% / -4.1% |
| 32 user-role copy-stress rows / backup | 3.953 → 3.651 (-7.6%) | 3.887 → 3.668 (-5.6%) | -1.9% / -0.7% |
| 4,096 user-role copy-stress rows / primary | 61.960 → 53.758 (-13.2%) | 62.456 → 54.075 (-13.4%) | -13.2% / -12.7% |
| 4,096 user-role copy-stress rows / backup | 62.860 → 54.316 (-13.6%) | 62.981 → 54.375 (-13.7%) | -14.2% / -13.1% |
| 32 × 128 alternating user/assistant rows / primary | 39.289 → 34.254 (-12.8%) | 39.282 → 34.206 (-12.9%) | -11.7% / -13.1% |
| 32 × 128 alternating user/assistant rows / backup | 39.661 → 34.372 (-13.3%) | 39.641 → 34.611 (-12.7%) | -15.4% / -3.4% |
| One thread × 8,192 messages / primary | 18.791 → 16.275 (-13.4%) | 18.646 → 16.277 (-12.7%) | -11.7% / -12.2% |
| One thread × 8,192 messages / backup | 18.877 → 16.453 (-12.8%) | 18.647 → 16.500 (-11.5%) | -13.5% / -10.5% |
| 512 threads × 2 messages / primary | 47.985 → 48.645 (+1.4%) | 49.182 → 47.756 (-2.9%) | +0.1% / -3.0% |
| 512 threads × 2 messages / backup | 48.329 → 49.404 (+2.2%) | 49.228 → 48.439 (-1.6%) | +1.6% / -1.6% |
| ASCII / compact / primary | 7.882 → 7.482 (-5.1%) | 7.460 → 7.404 (-0.7%) | -0.2% / -2.2% |
| ASCII / compact / backup | 8.006 → 7.738 (-3.3%) | 7.894 → 7.513 (-4.8%) | -5.6% / -5.9% |
| ASCII / pretty / primary | 8.150 → 7.668 (-5.9%) | 8.026 → 7.671 (-4.4%) | -1.5% / -5.1% |
| ASCII / pretty / backup | 8.226 → 7.766 (-5.6%) | 7.882 → 7.582 (-3.8%) | -6.3% / -2.4% |
| Unicode / compact / primary | 11.949 → 9.050 (-24.3%) | 11.750 → 9.148 (-22.1%) | -25.3% / -23.6% |
| Unicode / compact / backup | 12.124 → 9.182 (-24.3%) | 11.854 → 9.181 (-22.6%) | -25.7% / -22.4% |
| Unicode / pretty / primary | 12.501 → 9.521 (-23.8%) | 12.119 → 9.493 (-21.7%) | -26.5% / -21.6% |
| Unicode / pretty / backup | 12.344 → 9.605 (-22.2%) | 12.087 → 9.566 (-20.9%) | -23.2% / -20.2% |
| Escaped text / compact / primary | 13.008 → 11.632 (-10.6%) | 12.512 → 11.383 (-9.0%) | +22.6% / -6.8% |
| Escaped text / compact / backup | 12.909 → 11.737 (-9.1%) | 12.532 → 11.630 (-7.2%) | -9.6% / -10.5% |
| Escaped text / pretty / primary | 13.209 → 11.918 (-9.8%) | 12.817 → 11.785 (-8.0%) | -10.8% / -7.5% |
| Escaped text / pretty / backup | 13.366 → 12.004 (-10.2%) | 12.995 → 11.748 (-9.6%) | -11.8% / -7.9% |

All individual stages and percentiles remain in the raw comparison and interactive
view. Small controls, rendering costs and tails remain explicit. The largest total
p95 increase in this corpus is 8,192 transcript rows /
backup in the forward order:
13.396 → 16.504 ms
(+23.2%). No whole-action, universal startup or universal tail acceptance is
claimed. Early syntax errors in large files, other corrupt shapes, concurrent work,
peak memory, physical presentation and OS-cold loading still need dedicated evidence;
whole-buffer validation can add work before an early JSON error is discovered.

The quick gate passes: **2,418 Rust tests, 32 manual tests ignored, 38 site tests**,
formatting, native all-target/all-feature and WASM Clippy with `-D warnings`, site types
and benchmark self-checks. Four after-only MCP runs retain **194 complete response
fingerprints and byte counts** across generic/projected/finance/context reads. All
42 prior ledger entries have file pins affected by the new lockfile; they retain
their original timing/binary/source artifacts with explicit source/contract/protocol
revalidation. Prior latency results remain historical; they are not new paired latency
claims for those actions. The ledger now has **43 partial entries across 33 identities**.

Reproduction and evidence:

- `target/performance-decoder-load-after` is the optimized executable, SHA-256
  `d14659a2c24bca8b5b35d15cbfee05ce7d8ad49c1a0835dea49d87b9f3dbef45`. Original 1,064-input and final 1,066-input manifests/source
  copies, the scope/reference proof, dependency audit, build/gate receipts and
  compatibility files remain under `target/performance-decoder-load-*`.
- `python3 target/performance-decoder-load-four-runs.py` runs the guarded paired
  corpus; `python3 target/performance-decoder-load-compare.py` verifies and summarizes
  it. Four JSON/log pairs retain raw samples, commands, environments, timestamps,
  binary/harness/source hashes, file fingerprints and Cargo locks.
- `python3 target/performance-decoder-load-component.py` repeats the separate
  actual-type mechanism experiment; `python3 target/performance-decoder-load-revalidate.py`
  repeats MCP compatibility. The report, chart and evidence ledger name all boundaries.
- The source pins cover named Rust/config/MCP inputs; installed validator sources are
  separately pinned. They do not pin every asset/toolchain input. The prior 9555 native
  executable and phase25 raw observations remain unchanged as historical evidence.

The broad all-action objective remains active. Initial rendering with many threads,
remaining decode/escape/allocation and hash costs, error/concurrency variants, actual
font/app initialization, physical input/startup-to-paint, live network/provider/capture
work, attachment bytes and cold storage still need measured work.

## Native backup loading without an unused disk digest

Backup recovery no longer calculates a disk digest that it immediately discards. The
4,096-document backup is 37.0–37.3% faster from file read through Root completion and
34.5–35.1% faster through headless settling. The alternating-conversation backup shows
29.7–30.7% and 24.7–26.2% faster respectively. Both paths retain primary raw-byte version
hashing and the current SIMD decoder and owned record/thread restoration.

Only one production file changes. The two saved-file readers share the original
path validation, read, decode and error-handling body. A typed completion callback
returns decoded state for unversioned loads; the primary snapshot completion also
hashes the same raw bytes. No mutable mode or size threshold is introduced. The
backup caller still returns `DiskVersion::Untrusted`, and a missing primary still
returns `Missing` without adopting an existing backup. The decoder, full backup branch,
schemas, writers, conflict protection, runtime construction, record/thread ownership,
actions, wire formats and dependency versions retain their code.

Four regular contracts compare complete rich/Unicode/ASCII/escaped state and exact
reader/error/recovery results with the frozen implementation. They check that compact
and pretty bytes representing the same state have different primary versions, that
valid primary data takes precedence over a bad backup, and that missing/invalid/both-
invalid/directory/symlink combinations retain policy and leave files unchanged.
Recovered state remains untrusted and an attempted guarded write returns `Conflict`
without invoking its write callback or changing either file. Existing multi-process
stale-writer, lock, conflict, backup and durability checks pass in the full gate.

The frozen three-reader reference uses the current SIMD decoder and original backup
hashing; its method bodies equal the preserved pre-change source after renaming and
blank-line formatting. The shared read/decode body equals the old body after changing
only successful completion and its missing result shape. The primary wrapper maps
that missing result to the original `Missing` version. The actual constructor and
out-of-line observer remain byte-identical to the prior source. The previous corpus
loop is shared by both performance tests; its timing and full-state/queue/version/
composer/file assertions remain intact, with only selector/expected-loader/log-label
injection. All **36 prior input fingerprints and byte counts match**.

Four fresh runs use original / current / current / original in one optimized executable:
18 shapes × primary/corrupt-primary backup, five warmups and 30 accepted observations
per case. That is **4,320 continuous observations and 72 paired case/order checks**.
Both implementations use one shared observer, the same actual Workspace/Root
constructor and the current decoder/ownership code. The runner pins source, executable
and harness, clears all optional case selectors, checks for builders before launch,
and holds all four native/WASM debug/release Cargo profile locks. Accepted runs are
unprofiled. Per-iteration timestamps retain read/decode/hash, construction, headless
settling and complete total intervals; complete Root intervals are calculated from
correlated timestamps rather than adding independent stage medians. Full restored
snapshots, held queues, zero saves/no provider, rendered composer, file versions,
recovery and unchanged file bytes are checked outside timing.

Corrupt-primary → backup: continuous **file read → Root ready**, milliseconds.
Negative changes are faster.

| Shape | Original first, p50 | Current first, p50 | p95 change, both orders |
|---|---:|---:|---:|
| Settings control | 0.236 → 0.231 (-2.1%) | 0.237 → 0.286 (+20.8%) | +7.0% / +9.3% |
| 256 knowledge documents | 3.420 → 2.232 (-34.7%) | 3.456 → 2.348 (-32.1%) | -32.7% / -29.6% |
| 4,096 knowledge documents | 52.089 → 32.651 (-37.3%) | 52.013 → 32.773 (-37.0%) | -35.0% / -36.2% |
| 4,096 records per collection | 27.670 → 18.131 (-34.5%) | 27.923 → 18.379 (-34.2%) | -33.9% / -33.5% |
| 64 threads × 128 messages | 14.033 → 9.871 (-29.7%) | 14.034 → 10.025 (-28.6%) | -29.2% / -27.4% |
| 8,192 transcript rows | 8.019 → 5.776 (-28.0%) | 7.983 → 5.968 (-25.2%) | -25.2% / -24.5% |
| Mixed workspace | 32.072 → 21.334 (-33.5%) | 32.009 → 21.619 (-32.5%) | -31.8% / -32.6% |
| 32 user-role copy-stress rows | 0.607 → 0.446 (-26.6%) | 0.674 → 0.450 (-33.2%) | -44.4% / -65.9% |
| 4,096 user-role copy-stress rows | 46.805 → 32.583 (-30.4%) | 46.793 → 32.529 (-30.5%) | -27.6% / -30.3% |
| 32 × 128 alternating user/assistant rows | 28.203 → 19.556 (-30.7%) | 28.225 → 19.844 (-29.7%) | -28.3% / -27.1% |
| One thread × 8,192 messages | 12.625 → 8.839 (-30.0%) | 12.742 → 8.704 (-31.7%) | -29.5% / -30.1% |
| 512 threads × 2 messages | 12.375 → 11.297 (-8.7%) | 12.507 → 11.048 (-11.7%) | -7.6% / -7.3% |
| ASCII / compact | 3.294 → 1.820 (-44.7%) | 3.391 → 1.756 (-48.2%) | -41.4% / -48.3% |
| ASCII / pretty | 3.562 → 1.953 (-45.2%) | 3.583 → 1.864 (-48.0%) | -40.1% / -46.9% |
| Unicode / compact | 4.820 → 2.668 (-44.7%) | 4.967 → 2.691 (-45.8%) | -39.7% / -41.7% |
| Unicode / pretty | 5.078 → 3.035 (-40.2%) | 5.085 → 2.831 (-44.3%) | -30.6% / -43.2% |
| Escaped text / compact | 7.083 → 5.283 (-25.4%) | 7.088 → 5.010 (-29.3%) | -21.7% / -28.2% |
| Escaped text / pretty | 7.240 → 5.354 (-26.0%) | 7.395 → 5.201 (-29.7%) | -22.6% / -28.8% |

Corrupt-primary → backup: continuous **file read → headless settling**, milliseconds.

| Shape | Original first, p50 | Current first, p50 | p95 change, both orders |
|---|---:|---:|---:|
| Settings control | 2.551 → 2.553 (+0.1%) | 2.534 → 2.729 (+7.7%) | +1.9% / +4.1% |
| 256 knowledge documents | 6.028 → 4.775 (-20.8%) | 6.037 → 5.009 (-17.0%) | -15.7% / -16.1% |
| 4,096 knowledge documents | 55.810 → 36.220 (-35.1%) | 55.684 → 36.474 (-34.5%) | -32.2% / -33.8% |
| 4,096 records per collection | 32.817 → 22.837 (-30.4%) | 32.981 → 23.093 (-30.0%) | -28.7% / -29.5% |
| 64 threads × 128 messages | 18.440 → 14.086 (-23.6%) | 18.521 → 14.281 (-22.9%) | -23.8% / -15.2% |
| 8,192 transcript rows | 11.512 → 9.279 (-19.4%) | 11.535 → 9.393 (-18.6%) | -20.5% / -17.4% |
| Mixed workspace | 37.204 → 26.078 (-29.9%) | 37.017 → 26.466 (-28.5%) | -28.1% / -28.3% |
| 32 user-role copy-stress rows | 3.658 → 3.356 (-8.3%) | 3.784 → 3.427 (-9.4%) | -17.9% / -60.2% |
| 4,096 user-role copy-stress rows | 54.773 → 40.041 (-26.9%) | 54.465 → 39.889 (-26.8%) | -25.1% / -26.4% |
| 32 × 128 alternating user/assistant rows | 34.778 → 25.679 (-26.2%) | 34.566 → 26.021 (-24.7%) | -24.0% / -22.3% |
| One thread × 8,192 messages | 16.521 → 12.554 (-24.0%) | 16.558 → 12.439 (-24.9%) | -22.2% / -22.3% |
| 512 threads × 2 messages | 48.530 → 44.487 (-8.3%) | 48.436 → 45.074 (-6.9%) | -8.2% / -6.3% |
| ASCII / compact | 7.319 → 5.798 (-20.8%) | 7.568 → 5.755 (-24.0%) | -17.4% / -23.4% |
| ASCII / pretty | 7.768 → 5.960 (-23.3%) | 7.731 → 5.807 (-24.9%) | -22.4% / -27.9% |
| Unicode / compact | 9.231 → 6.712 (-27.3%) | 9.272 → 6.829 (-26.3%) | -22.9% / -22.5% |
| Unicode / pretty | 9.496 → 7.337 (-22.7%) | 9.449 → 6.944 (-26.5%) | -15.9% / -24.9% |
| Escaped text / compact | 11.518 → 9.700 (-15.8%) | 11.596 → 9.097 (-21.5%) | -13.1% / -19.7% |
| Escaped text / pretty | 11.832 → 9.690 (-18.1%) | 11.919 → 9.364 (-21.4%) | -15.0% / -21.7% |

Primary-file control: continuous **file read → headless settling**, milliseconds.

| Shape | Original first, p50 | Current first, p50 | p95 change, both orders |
|---|---:|---:|---:|
| Settings control | 2.509 → 2.558 (+2.0%) | 2.474 → 2.699 (+9.1%) | -2.1% / +8.6% |
| 256 knowledge documents | 6.146 → 5.905 (-3.9%) | 5.804 → 6.123 (+5.5%) | -3.6% / +6.1% |
| 4,096 knowledge documents | 55.116 → 55.330 (+0.4%) | 55.645 → 55.441 (-0.4%) | +0.1% / +0.3% |
| 4,096 records per collection | 32.445 → 31.997 (-1.4%) | 32.639 → 32.274 (-1.1%) | -0.5% / -1.4% |
| 64 threads × 128 messages | 18.256 → 17.688 (-3.1%) | 18.330 → 18.538 (+1.1%) | -2.3% / +1.3% |
| 8,192 transcript rows | 11.457 → 11.199 (-2.3%) | 11.431 → 11.062 (-3.2%) | +1.0% / -2.7% |
| Mixed workspace | 37.192 → 36.369 (-2.2%) | 36.972 → 36.437 (-1.4%) | -1.9% / -0.7% |
| 32 user-role copy-stress rows | 3.603 → 3.482 (-3.3%) | 3.557 → 3.532 (-0.7%) | -1.7% / -2.9% |
| 4,096 user-role copy-stress rows | 53.945 → 52.856 (-2.0%) | 54.163 → 53.103 (-2.0%) | -0.8% / +5.9% |
| 32 × 128 alternating user/assistant rows | 34.398 → 33.409 (-2.9%) | 34.453 → 33.477 (-2.8%) | +1.8% / +0.9% |
| One thread × 8,192 messages | 16.533 → 16.131 (-2.4%) | 16.311 → 16.102 (-1.3%) | +0.2% / -0.9% |
| 512 threads × 2 messages | 47.938 → 45.301 (-5.5%) | 47.874 → 45.447 (-5.1%) | -3.9% / -3.9% |
| ASCII / compact | 7.533 → 7.158 (-5.0%) | 7.475 → 7.149 (-4.4%) | -4.2% / -8.2% |
| ASCII / pretty | 7.642 → 7.442 (-2.6%) | 7.480 → 7.378 (-1.4%) | -4.3% / -5.4% |
| Unicode / compact | 9.191 → 8.757 (-4.7%) | 9.085 → 8.790 (-3.2%) | +0.1% / -1.1% |
| Unicode / pretty | 9.478 → 9.531 (+0.6%) | 9.584 → 9.115 (-4.9%) | +6.1% / -4.1% |
| Escaped text / compact | 11.427 → 11.540 (+1.0%) | 11.498 → 11.131 (-3.2%) | +3.3% / -3.0% |
| Escaped text / pretty | 11.733 → 11.995 (+2.2%) | 11.809 → 11.387 (-3.6%) | +1.4% / -3.6% |

All primary Root intervals, individual stages, percentiles and raw samples remain in
the comparison and interactive view. Primary controls, rendering costs and tails do
not establish a universal startup gain. The largest total p95 increase is
Settings control / primary, reverse order:
2.784 → 3.023 ms
(+8.6%). No whole-action or universal tail acceptance is claimed.

The quick gate passes: **2,422 Rust tests, 33 manual tests ignored, 38 site tests**,
formatting, native all-target/all-feature and WASM Clippy with `-D warnings`, site types
and benchmark self-checks. Four after-only MCP runs retain **194 complete response
fingerprints and byte counts** across generic, projected, finance and context reads.
The 5 prior file-pin entries affected by
these source changes retain original latency/binary/source evidence with explicit
source/contract/protocol revalidation. Their old timings remain historical, with no
new paired latency claim for those earlier optimizations. The ledger has **44 partial
entries across 33 identities**; none is promoted to a complete action.

Evidence and reproduction:

- The final executable is `target/performance-backup-load-after`, SHA-256
  `6fc2337015b479324418181486b1b825504a0240fa37322acb725ae18a450a76`. Original/final manifests and source copies pin
  **1,066/1,068 inputs**. The previous `d14659…bef45` executable, phase26 raw samples,
  separate mechanism experiment and dependency audit remain unchanged.
- `python3 target/performance-backup-load-four-runs.py` runs the guarded comparison;
  `python3 target/performance-backup-load-compare.py` verifies all inputs and summarizes
  results. Four JSON/log pairs retain commands, explicit environments, timestamps,
  source/binary/harness hashes, Cargo locks and all raw samples.
- Scope/reference/shared-symbol proofs, contract/Clippy logs, build/gate receipts,
  compatibility, prior-entry revalidation, the report and chart remain under
  `target/performance-backup-load-*`. Source pins cover named Rust/config/MCP inputs,
  not every asset/toolchain input.
- `python3 target/performance-backup-load-revalidate.py` repeats the after-only MCP
  checks. It does not create new paired API latency evidence.

The broad goal remains active. This evidence covers warm saved-file loading through
headless executor settling. It excludes physical presentation/input/startup-to-paint,
OS-cold files, actual DesktopState/font initialization, live network/provider/capture
work, attachment bytes and peak memory. Large early-invalid JSON, other error and
concurrency shapes, remaining parse/allocation/I/O/hash work and initial/repeated
rendering with many threads still require measured work.

## Native sidebar input and lazy flex minimum measurement

Native flex layout now skips a minimum-content measurement when a known minimum
already determines the result. Measured repeated input/redraw medians fall
31.9–40.7% across all six scenes in both process orders;
initial-loading controls and tails remain mixed. The original Taffy 0.10.1 expression used eager
`Option::unwrap_or`, calculating and discarding its fallback. The single library
change is `unwrap_or_else(|| { ... })`; the existing fallback body and returned
sizing values remain intact. Rust documents the eager/lazy distinction in
[Option](https://doc.rust-lang.org/std/option/enum.Option.html#method.unwrap_or).

The application uses a vendored copy of the same published Taffy version. The
published crate checksum is verified, all retained published files match except
that one expression, and the MIT license is copied from the exact upstream commit
`639f0acdd20f914f847abffbeaa903d66cae4882`. The normalized manifest is unchanged.
Root Cargo changes only the workspace exclusion and same-version path override;
full `(name, version, source)` lock keys verify **1,068 other package records
unchanged**. Features, dependencies and application workspace membership match.
No application styles, search/controller logic, cache policy, schema, persistence,
provider, capture, ownership or wire source changes. This layout-engine change
can affect other native surfaces, so selected scene checks do not exhaust all UI.

The exact-upstream all-feature suite passes **4,406 tests, 0 failed, 4 ignored**,
including 4,277 generated layout cases and five doctests. A separate same-executable
original/candidate mechanism probe covers 12 tree shapes, **1,440 layout-only
observations and 24 paired checks**, comparing every recorded Layout field for
every node bit-for-bit after each run. These data exclude tree construction and
destruction and do not count as native-action observations.

The new app fixture mounts the actual Workspace/Root and sidebar search. Six
deterministic scenes cover 8, 128, 512 and 2,048 threads, Chat/Code tabs, project
groups, active/pinned/settled/snoozed rows, open/closed settled sections, two heights
and Markdown/Unicode assistant content. Initial construction, resize and focus are
outside repeated-action timing. Each cycle performs: a notify/redraw; complete
`Saved` text input; select-all/backspace; complete `absentquartz` input; and another
select-all/backspace. Search timings include the complete simulated text sequence,
its actual handler and headless executor/frame settling. They are **not per-key
or physical input-to-paint** measurements. Redraw is a rendering component, not a
user action. The input sequence is fixed within a cycle.

Every step compares the complete restored snapshot and checks thread/active IDs,
exact query, no persistence requests and no streamed/queued turns. Geometry checks
also verify input focus and composer presence. The optimized 8/24/24-thread contract
has **15 exactly equal step traces** before/after, covering selector presence and
bounds, query/focus, input bytes and restored-state fingerprints. Many-thread
benchmark final geometry, focus and state also match exactly across all four runs.
Optional bounds for off-screen/non-rendered selectors remain optional on both sides;
this is not a whole-app pixel or every-layout-node comparison.

Both app executables use the unchanged release LTO/codegen settings and identical
application/controller/fixture/observer Rust sources. **They are separate compiled
before/after binaries**, because the crates.io package origin and one dependency
expression differ. This is not same-executable causal isolation; generated-code
and process noise remain limitations. Each retains one actual Workspace constructor,
one loader observer and one sidebar interaction timer. The small mechanism probe
provides the independent same-executable check. The previous phase27 executable and
all earlier raw observations remain historical and are not pooled into these gains.

Each family runs four fresh sequential processes: before / after / after / before.
The labels forward/reverse denote inverse **variant process order**, not reversed
scene/event order. Six scenes × five events × five warmups/30 samples give
**3,600 intervals and 60 paired case/order checks**: 2,880 input intervals and 720
redraw intervals. The unchanged actual-file loader corpus adds **4,320 continuous
intervals and 72 paired checks** over all 36 primary/recovery cases. All 36 prior
input SHA/byte counts match. Combined native acceptance: **7,920 intervals and
132 paired checks**, kept separate from the mechanism probe.

The runner pins both source archives, current source, selected executable and
harness; clears optional case/implementation selectors; checks for builders; and
holds four native/WASM debug/release Cargo profile locks. Accepted timings are
unprofiled. Application Rust sources are identical between builds; each pinned
input set stays fixed during its own build and measurement. A transient builder
was detected by the first post-timing compatibility preflight, which refused to
launch; the next bounded inspection found none and the unchanged guard was retried.
Preflight checks/profile locks do not establish machine-exclusive CPU or a complete
background-process history, so unrelated process load remains a limitation.

Repeated native input/redraw through complete headless settling, milliseconds.
Negative percentages are faster. chat-512-active/search-all: 260.561 → 169.460 (-35.0%) and 262.193 → 166.810 (-36.4%) ms in the two orders.

| Scene / event | Before process first, p50 | After process first, p50 | p95 change, both orders |
|---|---:|---:|---:|
| chat-8-active/redraw | 1.327 → 0.859 (-35.3%) | 1.330 → 0.858 (-35.5%) | -41.5% / -28.8% |
| chat-8-active/search-all | 9.468 → 6.233 (-34.2%) | 9.685 → 6.246 (-35.5%) | -31.4% / -34.3% |
| chat-8-active/clear-all | 2.706 → 1.719 (-36.5%) | 2.699 → 1.744 (-35.4%) | -36.6% / -36.1% |
| chat-8-active/search-none | 13.481 → 8.773 (-34.9%) | 13.428 → 8.857 (-34.0%) | -33.4% / -34.1% |
| chat-8-active/clear-none | 2.277 → 1.458 (-36.0%) | 2.287 → 1.500 (-34.4%) | -35.0% / -38.9% |
| chat-128-active/redraw | 8.721 → 5.454 (-37.5%) | 8.827 → 5.231 (-40.7%) | -35.5% / -34.1% |
| chat-128-active/search-all | 61.020 → 38.930 (-36.2%) | 61.729 → 38.330 (-37.9%) | -35.6% / -35.2% |
| chat-128-active/clear-all | 17.515 → 11.068 (-36.8%) | 17.910 → 10.972 (-38.7%) | -37.0% / -37.5% |
| chat-128-active/search-none | 29.516 → 19.510 (-33.9%) | 29.562 → 19.301 (-34.7%) | -31.5% / -34.2% |
| chat-128-active/clear-none | 10.064 → 6.553 (-34.9%) | 10.096 → 6.381 (-36.8%) | -34.7% / -31.8% |
| chat-512-active/redraw | 37.327 → 24.248 (-35.0%) | 37.135 → 23.954 (-35.5%) | -35.1% / -40.5% |
| chat-512-active/search-all | 260.561 → 169.460 (-35.0%) | 262.193 → 166.810 (-36.4%) | -36.6% / -40.5% |
| chat-512-active/clear-all | 75.656 → 48.830 (-35.5%) | 74.644 → 48.027 (-35.7%) | -48.1% / -39.0% |
| chat-512-active/search-none | 90.181 → 60.007 (-33.5%) | 90.335 → 59.444 (-34.2%) | -34.7% / -38.4% |
| chat-512-active/clear-none | 39.179 → 25.289 (-35.5%) | 38.889 → 25.152 (-35.3%) | -34.8% / -35.8% |
| chat-512-mixed-open/redraw | 39.078 → 24.693 (-36.8%) | 38.599 → 25.750 (-33.3%) | -50.8% / -29.5% |
| chat-512-mixed-open/search-all | 271.878 → 173.118 (-36.3%) | 268.658 → 177.672 (-33.9%) | -66.5% / -22.3% |
| chat-512-mixed-open/clear-all | 78.609 → 50.809 (-35.4%) | 77.528 → 52.143 (-32.7%) | -57.6% / -17.7% |
| chat-512-mixed-open/search-none | 93.169 → 61.525 (-34.0%) | 93.465 → 63.644 (-31.9%) | -65.8% / -24.8% |
| chat-512-mixed-open/clear-none | 41.273 → 25.738 (-37.6%) | 41.100 → 26.935 (-34.5%) | -72.1% / -30.2% |
| code-512-mixed-closed/redraw | 29.507 → 18.617 (-36.9%) | 29.237 → 18.372 (-37.2%) | -35.2% / -36.6% |
| code-512-mixed-closed/search-all | 264.197 → 171.011 (-35.3%) | 265.655 → 171.305 (-35.5%) | -35.9% / -36.0% |
| code-512-mixed-closed/clear-all | 69.797 → 45.243 (-35.2%) | 70.288 → 45.159 (-35.8%) | -33.9% / -34.5% |
| code-512-mixed-closed/search-none | 86.491 → 57.376 (-33.7%) | 86.204 → 56.903 (-34.0%) | -33.2% / -34.5% |
| code-512-mixed-closed/clear-none | 31.537 → 20.306 (-35.6%) | 31.198 → 20.066 (-35.7%) | -33.6% / -34.5% |
| code-2048-active/redraw | 174.814 → 106.720 (-39.0%) | 175.623 → 104.949 (-40.2%) | -38.4% / -41.0% |
| code-2048-active/search-all | 1219.094 → 751.746 (-38.3%) | 1243.524 → 739.046 (-40.6%) | -38.8% / -41.0% |
| code-2048-active/clear-all | 351.401 → 217.881 (-38.0%) | 357.115 → 213.276 (-40.3%) | -38.3% / -40.7% |
| code-2048-active/search-none | 381.468 → 241.867 (-36.6%) | 386.562 → 237.891 (-38.5%) | -38.8% / -38.3% |
| code-2048-active/clear-none | 176.904 → 109.362 (-38.2%) | 180.589 → 107.150 (-40.7%) | -38.4% / -41.5% |

All action controls and tails are retained. 0 of the 60
case/order p50 comparisons increase. The largest action p95 percentage change is
`chat-512-mixed-open/clear-all`, reverse: 81.653 →
67.165 ms (-17.7%).
No universal tail, frame-rate or whole-Chat acceptance is inferred.

Initial loading: continuous actual **file read → headless initial settling**, ms.
The archived 512-thread startup is a distinct scene from the active-sidebar fixture;
it retains its original saved data, viewport and initialization. Repeated-input
gains must not be treated as an initial-load guarantee.
The current saved-file decoder, primary byte-version hashing, backup recovery and
record/thread ownership are common to both executables. Correlated timestamps also
retain read/decode/hash, constructor, initial settle and file-read-through-Root;
complete intervals come from per-iteration sums, never summed independent medians.
Full snapshot/queue/no-save/no-provider/composer/version/recovery/file-byte checks
remain outside timing. Stage percentiles and samples stay in the comparison/chart.

| Scene / event | Before process first, p50 | After process first, p50 | p95 change, both orders |
|---|---:|---:|---:|
| settings-control/primary | 2.458 → 1.553 (-36.8%) | 2.602 → 1.692 (-34.9%) | -30.7% / -32.4% |
| settings-control/backup | 2.520 → 1.568 (-37.8%) | 2.556 → 1.586 (-38.0%) | -35.2% / -37.2% |
| knowledge-small/primary | 5.814 → 4.937 (-15.1%) | 5.889 → 4.990 (-15.3%) | -13.9% / -15.4% |
| knowledge-small/backup | 4.618 → 3.784 (-18.1%) | 4.574 → 3.629 (-20.7%) | -18.1% / -19.7% |
| knowledge-large/primary | 55.332 → 54.653 (-1.2%) | 55.515 → 54.645 (-1.6%) | -1.1% / -1.6% |
| knowledge-large/backup | 36.508 → 35.726 (-2.1%) | 36.829 → 35.976 (-2.3%) | -2.3% / -2.2% |
| records-large/primary | 32.364 → 31.639 (-2.2%) | 32.692 → 31.764 (-2.8%) | -2.8% / -1.8% |
| records-large/backup | 23.072 → 22.901 (-0.7%) | 23.234 → 22.750 (-2.1%) | -0.7% / -4.5% |
| history-control/primary | 18.204 → 17.672 (-2.9%) | 18.033 → 17.920 (-0.6%) | -0.3% / +1.6% |
| history-control/backup | 14.421 → 14.053 (-2.6%) | 14.316 → 14.066 (-1.7%) | -3.7% / -0.1% |
| meeting-control/primary | 11.091 → 10.444 (-5.8%) | 11.176 → 10.613 (-5.0%) | -5.8% / -3.9% |
| meeting-control/backup | 9.596 → 8.670 (-9.6%) | 9.409 → 8.843 (-6.0%) | -17.4% / -5.4% |
| mixed/primary | 36.763 → 37.196 (+1.2%) | 36.470 → 38.358 (+5.2%) | +2.0% / +25.1% |
| mixed/backup | 26.527 → 26.458 (-0.3%) | 26.544 → 26.180 (-1.4%) | -1.5% / +1.7% |
| thread-rich-small/primary | 3.509 → 2.767 (-21.1%) | 3.499 → 2.738 (-21.7%) | -11.3% / -22.0% |
| thread-rich-small/backup | 3.473 → 2.648 (-23.8%) | 3.629 → 2.573 (-29.1%) | -22.3% / -24.8% |
| thread-rich-history/primary | 53.075 → 54.259 (+2.2%) | 52.922 → 53.812 (+1.7%) | +1.9% / -0.4% |
| thread-rich-history/backup | 40.142 → 40.566 (+1.1%) | 41.630 → 40.807 (-2.0%) | -0.8% / -2.9% |
| thread-assistant-history/primary | 33.828 → 33.675 (-0.5%) | 33.868 → 33.692 (-0.5%) | +0.0% / -3.4% |
| thread-assistant-history/backup | 26.339 → 26.023 (-1.2%) | 26.412 → 26.236 (-0.7%) | +12.9% / -2.3% |
| thread-single-history/primary | 16.222 → 15.985 (-1.5%) | 16.529 → 16.072 (-2.8%) | +0.3% / -3.4% |
| thread-single-history/backup | 12.801 → 12.152 (-5.1%) | 12.559 → 12.264 (-2.3%) | -8.5% / -1.5% |
| thread-many-small/primary | 46.101 → 47.992 (+4.1%) | 46.452 → 48.493 (+4.4%) | +15.4% / -0.1% |
| thread-many-small/backup | 45.469 → 47.501 (+4.5%) | 45.887 → 48.906 (+6.6%) | +5.6% / +3.3% |
| ascii-compact/primary | 7.226 → 6.730 (-6.9%) | 7.309 → 6.902 (-5.6%) | -7.8% / -6.2% |
| ascii-compact/backup | 5.932 → 5.421 (-8.6%) | 5.992 → 5.500 (-8.2%) | -5.9% / -9.3% |
| ascii-pretty/primary | 7.621 → 7.038 (-7.6%) | 7.505 → 7.145 (-4.8%) | -5.9% / -3.9% |
| ascii-pretty/backup | 6.210 → 5.351 (-13.8%) | 6.061 → 5.430 (-10.4%) | -11.2% / -9.0% |
| unicode-compact/primary | 9.145 → 8.611 (-5.8%) | 8.812 → 8.457 (-4.0%) | -8.6% / -4.3% |
| unicode-compact/backup | 7.216 → 6.389 (-11.5%) | 6.697 → 6.360 (-5.0%) | -13.4% / -3.2% |
| unicode-pretty/primary | 9.170 → 8.824 (-3.8%) | 9.089 → 8.952 (-1.5%) | -3.9% / -1.1% |
| unicode-pretty/backup | 6.912 → 6.382 (-7.7%) | 6.810 → 6.549 (-3.8%) | -7.5% / -4.2% |
| escaped-compact/primary | 10.992 → 11.097 (+1.0%) | 11.287 → 11.108 (-1.6%) | +0.8% / -2.4% |
| escaped-compact/backup | 9.187 → 8.966 (-2.4%) | 9.228 → 8.973 (-2.8%) | -1.3% / -2.0% |
| escaped-pretty/primary | 11.510 → 11.015 (-4.3%) | 11.787 → 11.252 (-4.5%) | -3.1% / -14.7% |
| escaped-pretty/backup | 9.287 → 9.122 (-1.8%) | 9.310 → 9.295 (-0.2%) | +1.9% / +2.3% |

10 of the 72 initial-loading case/order p50 comparisons increase.
The 512-thread primary startup is 46.101 → 47.992 (+4.1%)
and 46.452 → 48.493 (+4.4%) ms. Its backup
startup is 45.469 → 47.501 (+4.5%) and
45.887 → 48.906 (+6.6%) ms. Any increases remain
a concrete follow-up, despite the large repeated-input improvements.
The largest initial-loading total p95 increase is `mixed/primary`,
reverse: 37.855 →
47.348 ms (+25.1%).
Read/constructor controls and tails remain explicit; this is no universal startup
or file-decoder speedup claim.

The app quick gate passes: **2,424 Rust tests, 34 manual tests ignored, 38 site
tests**, formatting, native all-target/all-feature and WASM Clippy with `-D warnings`,
site types and benchmark self-checks. Four after-only MCP families preserve
**194 complete response fingerprints and byte counts**. All **44 prior entries**
have file-level pins affected by Cargo metadata (and some by the new test-module
declaration). Their original latency/source/binary evidence remains historical;
explicit source/contract/protocol revalidation records the layout dependency change,
upstream and selected native geometry coverage, full gate and response compatibility.
It does not establish current paired latency for those older optimizations. The
ledger now has **45 partial entries across 33 identities**, with no complete-action
promotion. The source catalogue remains a discovery aid.

Evidence and reproduction:

- Before optimized binary: `target/performance-sidebar-layout-before`, SHA-256
  `d02bdf4802c7928afb6ef4bb478fadd34bd68555b03adb6533ae81b053317428`.
  After optimized binary: `target/performance-sidebar-layout-after`, SHA-256
  `e196a60ebf08bd00becf756b4ebba793ebc57d046b7a2d15a5b1fb3870f65907`. Original/before/after source manifests and
  copies retain **1,068 / 1,069 / 1,135 inputs**. The prior phase27 executable is
  `6fc2337015b479324418181486b1b825504a0240fa37322acb725ae18a450a76` and remains unchanged.
- `python3 target/performance-sidebar-layout-batch.py --cohort NAME` repeats all
  guarded comparisons in a new directory without overwriting the accepted cohort.
  Eight JSON/log pairs retain commands, environment, timestamps, raw samples,
  source/binary/harness hashes and locks. The comparison script recomputes p50 as
  median and p95 as nearest-rank ceil(0.95×30), preserves every case and verifies
  all fixture/geometry and correlated-stage equality checks.
- Scope, published-crate/upstream provenance, source archives, build/gate/native
  geometry receipts, symbol checks, the separate probe and conformance log,
  compatibility, prior-entry revalidation and interactive comparison remain under
  `target/performance-sidebar-layout-*`. Source manifests cover named source/config
  inputs and the vendored crate, not every asset, external dependency or toolchain input.
- `python3 target/performance-sidebar-layout-revalidate.py --cohort NAME` repeats after-only
  compatibility checks. It creates no new paired MCP latency evidence.

The goal remains active. Physical presentation/input/startup-to-paint, OS-cold files,
actual DesktopState/font initialization, live providers/network/capture, attachment
bytes and peak memory remain outside this acceptance. Sidebar scrolling, thread
selection, section toggling, other views/distributions and error/concurrency paths
still require measurements. Investigate the archived 512-thread startup tradeoff
before broader initial-loading acceptance. The largest scenes still mount many rows and make
multiple native frames for full query input; those remaining costs need profiling.
Large early-invalid JSON and the broader all-action audit remain open.

## Restored chat construction without discarded directory lookup

Restored chats now use their already-resolved project path during construction.
Across the two run orders and primary/recovery routes, the 512-thread fixture
shows **9.6–13.4% lower warm p50 from file read through headless settling**
and 45.0–48.3% lower p50 through Root completion. Small controls and
tails remain explicit below.

The native sample located `ChatView::new` calling `std::env::current_dir`, with
`getcwd`/open/stat underneath it during restored-thread construction. The caller
already had a saved project path and immediately overwrote that default. Production
now passes that known path into the constructor, for both fresh and restored threads.
The constructor body is otherwise identical. Existing `set_project_context` still
runs, retaining its project metadata, git and mention-hint behavior. Saved empty/
missing-path fallback is unchanged; standalone default-directory construction remains
available for tests and the controlled reference. No process-directory cache, path
validation or filesystem policy is introduced. Actual fresh-thread action latency
has not been accepted by this saved-file comparison.

The earlier Taffy startup tradeoff was investigated before editing source. A new
isolated repeat covers three shapes × primary/backup in four ordered processes:
**720 native intervals / 12 paired checks**. An additional full 36-case repeat
retains the original sequence: **4,320 intervals / 72 paired checks**. The archived
512-thread total changes are -2.6% to -0.9% in isolation, and -2.7% to +1.0% in the
full repeat, compared with +4.1% to +6.6% in the original phase28 cohort. That
slowdown is not stable across cohorts; the original results and mixed-workspace
tails remain preserved. These **5,040 diagnostic intervals and 84 paired checks**
stay separate from each other and from the new optimization's evidence. The
directory lookup is a separately demonstrated bottleneck, not a proven cause of
the earlier Taffy comparison's variation.

The current/lazy sample has a native call graph through the redundant cwd lookup.
The eager sample command exited successfully but returned an empty call graph;
it is excluded from call-path comparisons. Both profiled runs' **120 intervals**
are excluded from accepted timings. The fixture also performs untimed model
validation and teardown, so whole-fixture sampling percentages are not assigned to
the timed startup boundary. The working directory for this evidence is the
repository path recorded in the runner; directory depth/filesystem state can change
the size of a discarded cwd lookup. Physical app initialization remains separate.

The new primary comparison uses **one optimized executable**. A frozen owned
restorer and thread factory equal the prior production methods after names and
formatting, keeping their default-directory constructor call. Both variants use
the same ChatView construction body, actual Workspace/Root constructor, out-of-line
observer, file reader/current SIMD decoder, primary raw-byte hash/backup recovery,
owned record/message/queue transfer and current Taffy layout code. The reference's
cwd default and the production known-path call are the measured distinction. All
36 measured cases (18 shapes × primary/backup) contain restored threads; this does not establish
latency for an absent-state/fresh-thread fallback. The older `e196a6…65907` binary
is preserved and is not pooled into the new gains.

The existing full-file measurement body now accepts two typed thread restorers;
its legacy wrappers pass current/current. Its timing, full saved-state, held-queue,
zero-save/no-stream, composer, primary-version/recovery and unchanged-file-byte
checks remain intact. Every variant uses the same current loader and expected
frozen-owned state. All **36 prior input fingerprints and byte counts match**.
Root completion is calculated from correlated per-iteration read+construct values,
and total = read + construct + settle, never summed independent medians.

Four fresh processes run default / known / known / default, with five warmups and
30 accepted samples per shape/route: **4,320 continuous intervals and 72 paired
case/order checks**. The runner pins source/binary/harness, clears every optional
case/implementation selector, checks for builders at each preflight/postflight and
holds four native/WASM debug/release Cargo profile locks. Accepted runs are
unprofiled. Locks and process checks do not prove machine-exclusive CPU or rule
out background load between checks.

Continuous warm actual **file read → Root completion**, milliseconds.
Negative percentages are faster. Every arrow shows default-directory → known-path
latency, regardless of process order. The p95 percentages use default-first /
known-first process order.

| Shape / route | Default directory first, p50 | Known path first, p50 | p95 change, default first / known first |
|---|---:|---:|---:|
| settings-control/primary | 0.258 → 0.375 (+45.1%) | 0.232 → 0.181 (-22.0%) | +712.6% / -23.7% |
| settings-control/backup | 0.255 → 0.418 (+64.1%) | 0.265 → 0.188 (-29.0%) | +1611.9% / -31.7% |
| knowledge-small/primary | 3.337 → 3.736 (+12.0%) | 3.306 → 3.283 (-0.7%) | +372.5% / -1.3% |
| knowledge-small/backup | 2.162 → 2.782 (+28.7%) | 2.177 → 2.156 (-0.9%) | +231.4% / -4.0% |
| knowledge-large/primary | 49.482 → 52.498 (+6.1%) | 51.860 → 50.932 (-1.8%) | +32.2% / +0.0% |
| knowledge-large/backup | 32.029 → 32.859 (+2.6%) | 32.358 → 32.355 (-0.0%) | +4.8% / -0.2% |
| records-large/primary | 26.434 → 27.292 (+3.2%) | 27.262 → 27.268 (+0.0%) | +3.4% / +0.3% |
| records-large/backup | 18.139 → 18.405 (+1.5%) | 18.306 → 18.467 (+0.9%) | +0.7% / -1.1% |
| history-control/primary | 13.429 → 12.998 (-3.2%) | 13.720 → 13.000 (-5.3%) | -6.5% / -6.1% |
| history-control/backup | 9.926 → 9.302 (-6.3%) | 9.998 → 9.412 (-5.9%) | -7.3% / -4.8% |
| meeting-control/primary | 7.735 → 7.697 (-0.5%) | 7.706 → 7.670 (-0.5%) | -6.1% / -2.1% |
| meeting-control/backup | 5.809 → 5.917 (+1.9%) | 5.906 → 5.737 (-2.8%) | +1.2% / -2.3% |
| mixed/primary | 30.404 → 31.090 (+2.3%) | 31.296 → 31.354 (+0.2%) | +0.0% / +0.4% |
| mixed/backup | 21.143 → 21.235 (+0.4%) | 21.398 → 21.189 (-1.0%) | -0.5% / -0.4% |
| thread-rich-small/primary | 0.523 → 0.512 (-2.2%) | 0.547 → 0.537 (-1.8%) | -17.5% / -9.9% |
| thread-rich-small/backup | 0.418 → 0.419 (+0.3%) | 0.436 → 0.455 (+4.3%) | -3.2% / -17.9% |
| thread-rich-history/primary | 43.877 → 44.211 (+0.8%) | 45.442 → 45.389 (-0.1%) | +0.3% / -4.4% |
| thread-rich-history/backup | 32.450 → 32.342 (-0.3%) | 32.923 → 33.203 (+0.8%) | -1.5% / -1.1% |
| thread-assistant-history/primary | 26.644 → 26.378 (-1.0%) | 27.603 → 27.547 (-0.2%) | -0.8% / -1.2% |
| thread-assistant-history/backup | 19.942 → 19.607 (-1.7%) | 20.785 → 20.196 (-2.8%) | -4.1% / -9.3% |
| thread-single-history/primary | 12.472 → 12.363 (-0.9%) | 13.038 → 12.585 (-3.5%) | +1.7% / -10.2% |
| thread-single-history/backup | 10.041 → 8.606 (-14.3%) | 8.947 → 8.707 (-2.7%) | -20.1% / -7.1% |
| thread-many-small/primary | 11.948 → 6.173 (-48.3%) | 11.718 → 6.405 (-45.3%) | -48.7% / -44.3% |
| thread-many-small/backup | 11.080 → 6.090 (-45.0%) | 11.960 → 6.326 (-47.1%) | -40.9% / +16.8% |
| ascii-compact/primary | 3.175 → 3.124 (-1.6%) | 3.264 → 3.547 (+8.6%) | +1.4% / +196.5% |
| ascii-compact/backup | 1.849 → 1.665 (-10.0%) | 1.809 → 2.152 (+18.9%) | -5.1% / +105.4% |
| ascii-pretty/primary | 3.479 → 3.284 (-5.6%) | 3.375 → 3.944 (+16.9%) | -4.5% / +115.4% |
| ascii-pretty/backup | 1.979 → 1.771 (-10.5%) | 1.820 → 2.121 (+16.6%) | -9.7% / +25.7% |
| unicode-compact/primary | 4.800 → 4.585 (-4.5%) | 4.723 → 4.643 (-1.7%) | -9.4% / +1.5% |
| unicode-compact/backup | 2.699 → 2.591 (-4.0%) | 2.550 → 2.689 (+5.5%) | -3.8% / +8.3% |
| unicode-pretty/primary | 4.901 → 4.785 (-2.4%) | 4.978 → 4.995 (+0.3%) | -1.2% / +1.8% |
| unicode-pretty/backup | 2.804 → 2.682 (-4.4%) | 2.746 → 2.825 (+2.8%) | -1.6% / +9.3% |
| escaped-compact/primary | 6.983 → 6.898 (-1.2%) | 6.973 → 6.989 (+0.2%) | -1.7% / +3.0% |
| escaped-compact/backup | 4.893 → 4.849 (-0.9%) | 4.968 → 4.963 (-0.1%) | +0.2% / +10.7% |
| escaped-pretty/primary | 7.309 → 7.187 (-1.7%) | 7.251 → 7.279 (+0.4%) | +0.7% / +2.5% |
| escaped-pretty/backup | 5.253 → 5.052 (-3.8%) | 5.087 → 5.211 (+2.4%) | -0.8% / +8.5% |

Continuous warm actual **file read → headless initial settling**, milliseconds.
Individual read, constructor and settle percentiles and raw samples stay in the
comparison and interactive view.

| Shape / route | Default directory first, p50 | Known path first, p50 | p95 change, default first / known first |
|---|---:|---:|---:|
| settings-control/primary | 1.842 → 9.402 (+410.4%) | 1.636 → 1.526 (-6.7%) | +1625.3% / -11.2% |
| settings-control/backup | 1.678 → 4.894 (+191.6%) | 1.730 → 1.531 (-11.5%) | +994.0% / -13.0% |
| knowledge-small/primary | 4.892 → 6.326 (+29.3%) | 4.900 → 4.823 (-1.6%) | +613.8% / -4.0% |
| knowledge-small/backup | 3.743 → 5.397 (+44.2%) | 3.767 → 3.735 (-0.9%) | +648.5% / -3.6% |
| knowledge-large/primary | 52.031 → 55.354 (+6.4%) | 54.517 → 53.650 (-1.6%) | +31.6% / -0.0% |
| knowledge-large/backup | 34.639 → 35.704 (+3.1%) | 35.152 → 35.118 (-0.1%) | +5.2% / +0.0% |
| records-large/primary | 30.156 → 31.063 (+3.0%) | 31.032 → 31.102 (+0.2%) | +3.9% / +1.6% |
| records-large/backup | 21.919 → 22.373 (+2.1%) | 22.095 → 22.410 (+1.4%) | +0.5% / -0.7% |
| history-control/primary | 16.727 → 16.608 (-0.7%) | 17.262 → 16.571 (-4.0%) | -2.8% / -4.1% |
| history-control/backup | 13.394 → 12.869 (-3.9%) | 13.610 → 13.104 (-3.7%) | -4.7% / -4.7% |
| meeting-control/primary | 10.260 → 10.103 (-1.5%) | 10.169 → 10.303 (+1.3%) | -3.3% / -1.5% |
| meeting-control/backup | 8.430 → 8.466 (+0.4%) | 8.463 → 8.318 (-1.7%) | -0.4% / -3.3% |
| mixed/primary | 34.389 → 35.383 (+2.9%) | 35.247 → 35.487 (+0.7%) | +1.2% / +1.0% |
| mixed/backup | 25.325 → 25.490 (+0.6%) | 25.607 → 25.472 (-0.5%) | -0.4% / -2.6% |
| thread-rich-small/primary | 2.426 → 2.462 (+1.5%) | 2.493 → 2.550 (+2.3%) | -3.2% / -4.2% |
| thread-rich-small/backup | 2.271 → 2.341 (+3.1%) | 2.369 → 2.416 (+2.0%) | -2.6% / -2.7% |
| thread-rich-history/primary | 50.008 → 50.264 (+0.5%) | 51.692 → 51.997 (+0.6%) | +1.5% / -4.4% |
| thread-rich-history/backup | 38.714 → 38.589 (-0.3%) | 39.241 → 39.714 (+1.2%) | -0.8% / -0.5% |
| thread-assistant-history/primary | 31.632 → 31.447 (-0.6%) | 32.585 → 32.872 (+0.9%) | -1.4% / +0.9% |
| thread-assistant-history/backup | 25.082 → 24.758 (-1.3%) | 26.328 → 25.541 (-3.0%) | -0.9% / -10.5% |
| thread-single-history/primary | 15.480 → 15.215 (-1.7%) | 16.901 → 15.607 (-7.7%) | +2.6% / -17.2% |
| thread-single-history/backup | 14.458 → 11.620 (-19.6%) | 12.186 → 11.809 (-3.1%) | -26.0% / -5.8% |
| thread-many-small/primary | 44.881 → 38.863 (-13.4%) | 44.642 → 40.266 (-9.8%) | -13.7% / -8.5% |
| thread-many-small/backup | 44.008 → 39.765 (-9.6%) | 45.389 → 41.035 (-9.6%) | -7.3% / +231.2% |
| ascii-compact/primary | 6.463 → 6.527 (+1.0%) | 6.601 → 16.789 (+154.3%) | +0.1% / +492.0% |
| ascii-compact/backup | 5.278 → 5.084 (-3.7%) | 5.300 → 8.174 (+54.2%) | -5.2% / +166.3% |
| ascii-pretty/primary | 6.869 → 6.666 (-3.0%) | 6.862 → 8.316 (+21.2%) | -4.3% / +87.3% |
| ascii-pretty/backup | 5.487 → 5.162 (-5.9%) | 5.071 → 5.995 (+18.2%) | -1.9% / +27.2% |
| unicode-compact/primary | 8.307 → 8.087 (-2.7%) | 8.228 → 8.417 (+2.3%) | -5.3% / +7.3% |
| unicode-compact/backup | 6.054 → 6.055 (+0.0%) | 5.843 → 6.195 (+6.0%) | -0.7% / +9.6% |
| unicode-pretty/primary | 8.416 → 8.228 (-2.2%) | 8.436 → 8.737 (+3.6%) | -1.4% / +4.5% |
| unicode-pretty/backup | 6.219 → 6.122 (-1.6%) | 6.065 → 6.344 (+4.6%) | -0.7% / +8.9% |
| escaped-compact/primary | 10.351 → 10.348 (-0.0%) | 10.512 → 10.867 (+3.4%) | -0.8% / +6.0% |
| escaped-compact/backup | 8.217 → 8.448 (+2.8%) | 8.511 → 8.897 (+4.5%) | +3.2% / +9.9% |
| escaped-pretty/primary | 10.755 → 10.612 (-1.3%) | 10.748 → 11.126 (+3.5%) | +0.9% / +4.3% |
| escaped-pretty/backup | 8.727 → 8.511 (-2.5%) | 8.611 → 8.809 (+2.3%) | +3.5% / +7.6% |

38 of the 72 total p50 comparisons increase. The largest total p95 increase
is `settings-control/primary`, forward: 2.252 →
38.861 ms (+1625.3%). Controls and tails do not
establish a universal startup, rendering or decoder improvement. Large settling
outliers appear in the first known-path process at settings/knowledge and in the
second known-path process around the 512-thread backup/ASCII samples. A separate **240-interval / four
paired-check** settings repeat uses the same first-case prefix; total median
changes range from -0.5% to +8.7%, and the earlier hundreds-of-percent increases do
not repeat there. That does not identify their cause or erase the original data.
Both cohorts remain visible, and large settling tails require further diagnosis. This change is retained as a scoped construction improvement: the 512-thread
constructor p50 drops in both orders and routes, and complete warm p50 improves
there. It removes a demonstrated redundant lookup while state/behavior contracts
pass. It does **not** accept overall startup performance: the control and tail
regressions remain unresolved performance issues, and the control repeat does not
exonerate them. Rendering/layout source is unchanged; many-thread initial layout
remains a substantial cost.

The new full-snapshot path contract compares current and frozen-owned construction
for absolute Unicode, relative, whitespace, empty and absent project paths through
primary/recovery loading. Existing session/permission/model, messages/tools/queue
ownership and legacy timestamp/fallback contracts also pass. The optimized native
geometry contract retains **15 exactly equal step traces** against phase28. One
after-only repeated sidebar run checks **900 intervals**, with all full model/query/
no-save/no-stream assertions and **30 final geometry/focus cases matching** the
prior corpus through 2,048 threads. These rows are retained, with no new paired
repeated-input latency claim. Debug selector smoke contributes 120 excluded timings.

The quick gate passes **2,425 Rust tests, 35 manual tests ignored, 38 site tests**,
formatting, native all-target/all-feature and WASM Clippy with `-D warnings`, site
types and benchmark self-checks. Four after-only MCP families retain **194 complete
response fingerprints and byte counts**. Source scope verifies two production
modules and additive/shared test infrastructure; Cargo, Taffy, layout/handlers,
persistence/decoder, provider/capture and transport sources are unchanged. The
15 affected prior entries get explicit
source/contract/protocol revalidation; their previous source/binary/timing data
remain historical. The ledger has **46 partial entries across 33 identities**,
with no complete-action promotion.

Evidence and reproduction:

- Current optimized executable `target/performance-sidebar-startup-after`, SHA-256
  `626bd23ea12aa1bc38180212cfa85644919638815c80fefdfd0c3655db1bb7a5`. Original/before/after manifests and source copies retain
  **1,135 / 1,135 / 1,137 inputs**. The before manifest exactly copies the phase28
  after manifest; its reused-build receipt checks all archived source hashes.
- `python3 target/performance-sidebar-startup-four-runs.py --cohort NAME` repeats
  the guarded primary comparison in a new directory. Raw JSON/logs retain commands,
  environments, correlated samples, timestamps and source/binary/harness/lock proof.
  `python3 target/performance-sidebar-startup-compare.py` independently verifies
  inputs and computes median p50 / nearest-rank p95. Accepted-cohort files are
  never silently overwritten by these runners.
- Source/frozen-reference/shared-symbol proofs, source archives, sampled/isolated/
  full-repeat diagnostics, build/gate/debug/native/MCP receipts, the chart and prior
  revalidation stay under `target/performance-sidebar-startup-*`. Four full-repeat
  raw runs remain in `target/performance-sidebar-layout-extra-cohorts/phase29-startup-recheck`.
  Source pins cover named source/config/dependency inputs, not every asset or toolchain input.
- `python3 target/performance-sidebar-startup-native-revalidate.py --cohort NAME`
  and `python3 target/performance-sidebar-startup-revalidate.py --cohort NAME`
  repeat after-only native and MCP checks without creating paired action/API gains.

The broad goal remains active. Native layout with many mounted rows, sidebar
scrolling/selection/section toggles, actual fresh-thread latency, early-invalid JSON,
other error/concurrency/memory distributions and remaining semantic actions need
measured work. OS-cold files, real DesktopState/font/auth initialization, physical
input/startup-to-paint, live providers/network/capture, attachment bytes and peak
memory remain outside this acceptance.

## Workspace measurement isolation and CPU diagnosis

The full-workspace benchmark now finishes the previous fixture's entity cleanup
before another timed mount. This corrects the measurement boundary; it does not
change production application behavior. Earlier total/settle figures from this
observer include work from the previous fixture and remain historical with that
qualification. The actual native font/input/paint boundary still needs measurement.

Two independent debug probes establish the problem. In 18 observations, removing
the window, dropping external Root/Workspace handles and draining the test executor
leaves both release callbacks pending. An explicit App update releases them.
A further 18 observations mount the next actual Workspace/Root: the previous
entities remain pending at builder entry and at Root construction completion,
then release before that mount returns. GPUI's installed source confirms that
App effect flushing drains the dropped-entity queue. The old settling interval
therefore includes previous-fixture cleanup. This does not prove the cause of
every old tail, and it does not apply the same contamination claim to a repeated
action measured within one already-mounted window.

Both full-workspace observers now share an untimed teardown helper:

1. Remove the window.
2. Drop the external Root and Workspace handles.
3. Flush App effects.
4. Drain the executor.

The new lifecycle contract builds real Root/Workspace entities through primary
and recovery loading with one and 512 saved threads. It compiles and fails with
the old teardown, then passes all eight release checks with the repair. Timing
statements, file factories, constructor behavior and production sources are
unchanged. No samples are silently edited or corrected by subtracting cleanup.

The installed Workspace TestAppContext also uses **NoopTextSystem and no GPU
renderer**. These particular fixtures execute real application model, element,
text-metric and layout logic with dummy text metrics. They do not execute the
production platform's font shaping or GPU presentation. Earlier headless
workspace figures must be read within that scope; this finding is not generalized
to native component fixtures that use a different text backend.

Two separately archived optimized executables retain the original and repaired
measurement boundaries. Each runs the same 36 saved-file inputs in four
default-directory / known-path / known-path / default-directory processes for
each family. The 36 byte lengths and SHA-256 fingerprints match phase29.
Each family and cohort uses five warmups and 30 measured observations per case.

Raw names `original` and `known` identify the constructor variant in both cohorts:
the default-directory reference and the production known-path constructor.
The first two processes form the default-first order group; the last two form
the known-first group. Each unprofiled order group compares all 36 cases, giving
72 case/order comparisons per cohort. Iteration IDs run from 0 to 34: warmups are 0–4,
and the 30 measured observations retain IDs 5–34.

| Family | Original teardown | Repaired teardown | Use |
|---|---:|---:|---|
| Unprofiled warm intervals | 4,320 | 4,320 | Constructor comparison within each cohort |
| Instrumented warm intervals | 4,320 | 4,320 | Correlated elapsed, thread/process CPU and resource diagnosis |
| Excluded warmups | 720 per family | 720 per family | Retained separately from warm statistics |
| Capture calibration calls | 4,000 | 4,000 | Standalone counter-call cost only |

The families and cohorts are never pooled into one latency distribution or one
speedup. Each unprofiled cohort retains 72 case/order comparisons. Full restored
state, held queues, zero-save/no-stream, composer, version/recovery and unchanged
file-byte assertions pass. Runner receipts pin source/binary/harness, clear the
optional selectors, check for builders/other native benchmark workers and hold
four native/WASM debug/release Cargo profile locks. These controls do not prove
exclusive use of the machine.

The repaired unprofiled comparison rechecks the preceding known-path constructor
change. Values below are default-directory / known-path milliseconds, regardless
of run order. All 36 cases, every stage and both p50/p95 values remain in the raw
comparison and interactive view. The two settings cases revisit the earlier
control tails; the two 512-thread cases recheck the preceding constructor change.
They are selected diagnostics, not a representative aggregate of all cases.

| Shape / route | Total p50, default first | Total p50, known first | Constructor p50 change, default first / known first |
|---|---:|---:|---:|
| settings-control/primary | 1.442 / 1.446 (+0.3%) | 1.481 / 1.390 (-6.1%) | -10.3% / -19.6% |
| settings-control/backup | 1.536 / 1.497 (-2.6%) | 1.620 / 1.474 (-9.0%) | -13.2% / -23.6% |
| thread-many-small/primary | 25.328 / 19.428 (-23.3%) | 25.588 / 21.380 (-16.4%) | -63.0% / -53.7% |
| thread-many-small/backup | 25.915 / 18.750 (-27.6%) | 25.058 / 21.022 (-16.1%) | -64.2% / -53.0% |

26 of the 72 repaired total p50 comparisons increase. This retains the
construction optimization's scoped evidence while leaving controls, tails and
overall startup performance open. Moving cleanup outside the boundary is not a
production speedup, and cross-cohort timing differences are not used as one.

The separate CPU observer records elapsed/thread/process CPU and raw process
fault/context-switch counters at four stage boundaries. Each thread clock endpoint
must be on the same Rust/OS thread. The initial cohort's largest settling gap is
`ascii-pretty/backup`, `cpu-original-reverse`, iteration 34:
80.938 ms elapsed, 5.438 ms thread CPU,
5.489 ms process CPU. Its recorded process counter deltas are
0 minor/0 major faults and
0 voluntary/31 involuntary context switches.
The repaired cohort's largest gap is `escaped-pretty/backup`,
`cpu-original-forward`, iteration 31:
11.494 ms elapsed and 2.982 ms thread CPU.
These are identical-observation values, not differences between independent
percentiles. A large gap identifies elapsed time not accounted for by that
foreground thread's CPU clock; it does not identify a particular wait, OS cause
or the cause of a historical regression. Process counters include every thread.

The repaired 512-thread fixture still consumes foreground CPU during settling:

| Instrumented 512-thread fixture | Elapsed p50 ms | Thread CPU p50 ms | Same observation at elapsed p95, elapsed / thread CPU ms |
|---|---:|---:|---:|
| cpu-original-forward, primary | 14.264 | 14.190 | 15.468 / 15.449 |
| cpu-original-forward, backup | 14.299 | 14.158 | 16.300 / 16.279 |
| cpu-known-forward, primary | 14.983 | 14.871 | 16.384 / 16.276 |
| cpu-known-forward, backup | 15.143 | 15.001 | 16.703 / 16.595 |
| cpu-known-reverse, primary | 15.782 | 15.703 | 16.595 / 16.587 |
| cpu-known-reverse, backup | 15.847 | 15.771 | 17.302 / 17.202 |
| cpu-original-reverse, primary | 15.937 | 15.827 | 17.110 / 16.984 |
| cpu-original-reverse, backup | 16.152 | 16.089 | 17.291 / 17.175 |

The last column uses CPU from the exact observation selected by elapsed p95.
The two p50 columns are independently reported distributions and are not
subtracted to assign a cause. Four counter reads and an epoch marker perturb
instrumented observations. The 8,000 standalone calibrations describe capture
cost at those moments; their maxima are not per-observation overhead bounds.
Counter fields are read at slightly different instants. No overhead is subtracted,
and these instrumented values establish no unprofiled speedup.

The Xcode Time Profiler attempt reached its configured limit and ended the target
with SIGKILL, producing zero completed fixture rows. Its trace/receipt remain
inconclusive. An isolated Cargo probe confirms that a custom test harness can
retain cfg(test) and its own main; it has not launched or measured the real native
application. Both artifacts are preparation/diagnosis only.

The repaired quick gate passes **2,427 Rust tests and 38 site tests**, with 36
manual tests ignored. Formatting, native all-target/all-feature and WASM Clippy
with warnings denied, site types and benchmark self-checks pass. The initial
diagnostic gate retains its separate 2,426-test result. No MCP timing or response
revalidation is newly claimed in this phase: all production/transport sources
remain byte-identical. Six affected prior entries receive explicit source/contract
revalidation and measurement qualification. Prior raw timings stay historical.
The ledger now has 47 partial entries across 33 identities, with no whole-action
promotion.

Evidence and reproduction:

- The phase29 baseline remains `target/performance-settling-tail-before`, hash
  `626bd23ea12aa1bc38180212cfa85644919638815c80fefdfd0c3655db1bb7a5`.
  Initial diagnostic executable `target/performance-settling-tail-after`, hash
  `1f08f5200ea8758d3f6dc2bddc3c2fbe5fb7e92d5b2f2fbbe5f1ef96e2b35fc1`; repaired executable
  `target/performance-settling-tail-clean`, hash `badd74723265aaa307a9cee5f115f7b590b9ee233b7bed967fb04343bc1c3538`.
  Original/before/initial/repaired manifests and source copies retain
  1,137 / 1,137 / 1,138 / 1,139 named inputs. Assets/toolchain/runtime state
  are not comprehensively pinned by those source manifests.
- `python3 target/performance-settling-tail-clean-eight-runs.py --cohort NAME`
  repeats both repaired families without overwriting existing logs. Analyze the
  primary archived cohorts with `python3 target/performance-settling-tail-compare.py`
  and the same command with `--cohort clean`. The comparison uses median p50 and
  nearest-rank p95; raw iterations, warmups, calibration calls and timestamps remain.
- `target/performance-settling-tail-clean-change-scope.json` pins the installed
  GPUI backend and exact test-only source changes. Lifecycle probes, failing/passing
  contract, gate/build receipts, raw measurements, comparison chart, read-back and
  final verification stay under `target/performance-settling-tail-*`.

The broad goal remains active. The next native work must measure the production
font/input/frame boundary with synthetic state before accepting physical latency
claims, and profile the remaining many-row foreground work. Previous total/settle
optimization claims need repaired-fixture remeasurement before broader startup
acceptance. Scrolling/selection/toggles, fresh-thread latency, early-invalid JSON,
other semantic actions and error/concurrency/memory distributions remain open.
Real state/auth/providers/network/capture, OS-cold storage and peak memory are
outside this phase.

## Native platform baseline and the next sidebar fix

Real native measurements point to offscreen sidebar card construction as the next
optimization target. The validated application source is preserved; the sidebar
virtualization experiment is saved separately for the next pass. **There is no new
production speedup claim in this loop.** Work is paused at the user's request so
the laptop can be closed.

The new opt-in `bruhs-native-performance` binary uses the normal, non-headless
macOS platform and text system, embedded Monaspace fonts, the actual Workspace
and Root, and the production GPUI dependency features. Its linked artifacts have
neither `test-support` nor `bench`. This resolves the earlier fixture's dummy-text
limitation for these measurements.

Each process loads fresh synthetic pinned general threads into temporary saved
state. Credentials and Google token stores are in memory; startup auth, meeting,
preview and update services are explicitly blocked. Every observation checks the
complete model, query and thread order. The final acknowledged temporary save
matches the complete workspace snapshot. Save acknowledgement is outside the
action interval.

The baseline has 16 optimized processes across 1, 128, 512 and 2,048 threads,
with ascending and descending orders. It retains 1,680 warm wall observations and
1,680 separate CPU/frame observations; each family excludes 280 total warmup observations.
Seven cases cover three semantic action families plus a redraw control: whole-query
search updates, Previous Thread wrapping from first to last, and sidebar hide/show.
No physical keyboard, pointer, scrolling or live-provider action is timed.

The wall interval starts at synthetic handler dispatch and ends at the second
successive native frame callback. GPUI calls those callbacks before drawing, so
this brackets an intervening draw/present opportunity and includes callback cadence.
The CPU family requires a matching window draw inside that interval. Its draw time
covers CPU frame construction and excludes the subsequent GPU completion.

At 2,048 threads, the native wall medians are:

| Case | Ascending order, ms | Descending order, ms |
| --- | ---: | ---: |
| Redraw | 89.662 | 91.597 |
| Clear search | 89.021 | 89.265 |
| Previous Thread wrap | 91.622 | 92.518 |
| Show sidebar | 89.732 | 90.043 |
| Hide sidebar | 13.939 | 13.679 |
| Search, one match | 14.057 | 14.023 |
| Search, no match | 13.727 | 13.659 |

The separate instrumented family puts about 78–82 ms of median elapsed time in
CPU frame construction for the expensive cases. These independent medians are
not subtracted to assign a cause. A successful native callgraph sample shows
layout, element construction and painting under `Window::draw`; eager sidebar
construction includes all offscreen cards. The sample spans preparation as well as
action work and supplies call-path evidence, not a percentage of the timed interval.
All profiled observations remain excluded from the baseline.

**Resume point:** apply [the saved sidebar experiment](performance-sidebar-viewport-wip.patch)
against the restored native3 source, then validate it with normal native fonts and
optimized before/after runs. The experiment uses GPUI's variable-height list,
retains card action handlers and keyed focus targets, and traverses offscreen rows
with Tab. Its tests cover scrolling to the last card, forward/reverse keyboard
traversal, metadata height changes, closed-shelf search and clearing one match.
Correctness tests do not establish native rendering parity or a performance gain.
The experiment is not active in the application source at this stopping point.

The restored source is exactly the already-validated native3 manifest: 1,144 source
inputs and 30 embedded assets. Its quick gate passed 2,427 Rust tests (36 ignored)
and 38 site tests, including native all-target/all-feature and WASM Clippy. Four
additional feature-only harness tests passed. The default application entry point
still runs normally; the dedicated harness skips without explicit fixture arguments.

Evidence is under `target/performance-native-platform-*`: `comparison.json` contains
all baseline checks and per-order p50/p95 values; `profile-2048.sample.txt` contains
the resolved callgraph; `native3-manifest.json`, `native3-build-receipt.json`,
`gate-receipt.json` and `change-scope.json` pin source, assets and validation.
The saved experiment and its checkpoint receipt preserve the next implementation.
Physical input/presentation, initial startup, native scrolling, other thread shapes,
and the remaining application actions still need measurement.

## Native sidebar virtualization and frame-construction measurements

The desktop sidebar now renders visible cards and shelf headers while retaining
all threads and their actions. It also retains a focused offscreen row when needed. At 2,048 synthetic pinned threads, warm redraw,
clearing search, Previous Thread wrap and showing the sidebar improve from about
83–87 ms to 13–14 ms in both run orders. This is the synthetic dispatch-to-native-
callback interval, with callback cadence included. Physical input and GPU completion
remain outside the measurement.

This implementation is active in the working tree. It supersedes the separately
saved prototype at the November 10 stopping point above; that patch remains a
historical checkpoint.

**Behavior and decisions.** GPUI's variable-height list owns geometry; cards still
read current thread data and use their existing drawing and action handlers.
Focus handles stay keyed to each chat entity. Tab and Shift-Tab traverse every
logical card, its actions and delete control, including offscreen rows; claimed
custom keybindings retain precedence. Search, tab and project-filter changes
start at the first result, matching the web sidebar's existing viewport-filter effect.

Unmeasured rows receive height estimates from the actual visible layout after
GPUI's width-sensitive measurement. Their measured sizes replace those estimates
as they become visible. A failing wheel regression exposed that missing estimates
made the initial scrollbar cover only part of the list. The repaired test reaches
the final thread through a wheel event and then traverses all cards in both keyboard
directions. The estimate does not make every offscreen row's height exact immediately.

**Matched native results.** The comparison uses two equally optimized normal
native binaries, identical dependency features and assets, and exact synthetic
input bytes at 1, 128, 512 and 2,048 threads. Ascending runs execute before then
after; descending runs execute after then before. Thirty warm observations per
cell follow five excluded warmups.

| 2,048-thread case | Before ascending, ms | After ascending, ms | Before descending, ms | After descending, ms |
| --- | ---: | ---: | ---: | ---: |
| Redraw | 85.953 | 13.670 | 84.003 | 13.382 |
| Clear search | 83.772 | 13.644 | 82.786 | 13.645 |
| Previous Thread wrap | 84.698 | 13.644 | 86.872 | 13.644 |
| Show sidebar | 83.086 | 13.641 | 84.256 | 13.623 |

For these four cases, p95 falls from 86.680–93.241 ms to 13.768–15.612 ms across
the two orders. At 512 threads, the same cases' medians fall from 22.455–24.108 ms
to 13.628–14.110 ms. Small workspaces and already-cheap search/hide controls remain
near the callback floor; their tails are mixed. For example, the descending
2,048-thread one-match search p95 changes from 14.270 ms to 15.490 ms. These results
support the measured large-list improvement, not an every-action or every-tail claim.

The separate CPU/frame family shows the expensive cases' median frame-construction
elapsed time falling from 71.9–75.4 ms to 2.1–2.7 ms. Foreground thread CPU medians
fall from 75.5–79.5 ms to 2.4–3.0 ms. These are measurements from separate instrumented
runs, not subtractions from the primary wall intervals. The frame interval ends
before subsequent GPU completion.

All 32 native processes pass source/asset, model, query, thread-order, queue,
no-stream and frame checks. The final acknowledged temporary save matches the
complete snapshot, including across both binaries for each input size. The matrix
retains 3,360 warm wall observations and 3,360 separate CPU/frame observations;
each family excludes 560 warmups. Startup services stay explicitly blocked, and
credentials/Google state remain in memory. No live provider or capture action runs.

**Layout and correctness.** The optimized reference and normal-native debug candidate screenshots have identical RGB
pixels at 2,560 × 1,802 (a 1,280 × 901 logical viewport at 2×) for the inspected
pinned-thread state. Inspection caught the prototype's missing horizontal inset;
the repaired container restores the existing card layout. All screenshot and
inspection timings are excluded, including deliberately stopped or focus-failed runs.
This verifies one native reference state, rather than a fresh complete web/native
parity pass.

Separate NoopTextSystem component fixtures match 264 visible thread bounds and
60 surrounding control bounds across 15 query/focus/model steps. Another matched
fixture verifies every card, fold-header and control bound in two small mixed
open/closed-shelf scenes across ten steps. The old debug selector described an
overflowing content box; the new selector describes the viewport. Visible bounds
are compared within the footer-clipped viewport, and 354 offscreen debug bounds
are pruned. Supplemental tests were appended only to test fixtures in each archived
variant, then every active source byte was restored before native measurements.

The quick gate passes 2,429 Rust tests (36 ignored) and 38 site tests, plus native
all-target/all-feature Clippy, WASM Clippy, site types and benchmark self-checks.
The before manifest pins 1,144 source inputs; the after manifest pins 1,145. Both
use the same 30 embedded assets. Production changes are limited to sidebar geometry,
focus integration and a workspace field; the native fixture adds post-timing checks.
Thread-card drawing/handlers, persistence, thread ordering and the timing protocol
remain unchanged apart from explicit focus tracking and the described viewport work.

**Evidence status and remaining work.** This adds a current partial Chat variant.
The 48 earlier ledger entries retain their original source pins and measurements.
Twenty-six are marked source-changed by the new renderer and need revalidation
for their affected boundaries; their earlier startup/rendering timings are historical.
The new native result does not retroactively reaccept them.

Cold first use, initial workspace loading, large mixed/project/three-line card
workloads, streaming status updates, native scroll/fold timings, physical keyboard
and pointer input, GPU presentation, and the remaining application actions stay open.

Artifacts are under `target/performance-sidebar-viewport-*`: `comparison.json`
retains all orders, p50/p95 values and correlated p95 observations; `comparison.html`
provides action/metric/order controls; `change-scope.json`, `visual-verification.json`,
`geometry2-component-geometry.json`, `small-mixed-receipt.json` and `gate-receipt.json`
pin validation. The source/build/run manifests and raw native logs remain under
`target/performance-native-platform-*` (`native3`, `native5`, and `phase32-*`).
The optimized after binary SHA-256 is
`a792c5be67d1adea5a081e3d126898a1a2328bdf1531bbcc5dd91ea2cd124d31`.

## Shared initial Claude auth checks

Restored chats now share one initial Claude auth-status subprocess while its result
is pending foreground delivery. The isolated constructor fixture verifies these
process counts:

| Restored chats | Original checks | Shared initial check |
| --- | ---: | ---: |
| 1 | 1 | 1 |
| 8 | 8 | 1 |
| 32 | 32 | 1 |
| 128 | Not run | 1 |

These are verified operation counts. Full application startup latency remains
unmeasured for this change.

Each chat previously launched the same global `claude auth status` command during
construction, including synchronous saved-thread restoration. The command has no
project-specific working directory. The initial flight now belongs to the GPUI app
and keeps weak references to its waiting chats. Closing the originating chat still
allows delivery to the others. Foreground delivery, including a failed status,
clears the flight. Later chats start a new check because the CLI's account state
may have changed outside the app. Completed results are not cached.

Manual re-checks keep independent fresh probes. Claude model selection still calls
the same fresh-check method, confirmed by source inspection; the new fixture
exercises re-checks directly. Starting a fresh probe, or preparing interactive
login, invalidates that chat's pending initial
response so it cannot overwrite the newer state. The existing parser, command
arguments, timeout, login subprocess/polling logic and banner rendering are
unchanged. This adds no TTL, concurrency limit or dependency.

The fixture uses actual `ChatView::new_with_project_path` constructors and real
Tokio subprocess/output/parsing, with an explicit fake executable in a temporary
directory. GPUI uses its test platform and NoopTextSystem. Counter assertions cover
the original constructor path before the change. The final test build also reruns
the original one-probe-per-chat behavior. No account, provider or interactive login operation runs.
The login-state test exercises preparation only. Fixture waits are correctness
barriers and supply no accepted timing samples.

Eight tests cover process counts, fresh checks after completion, closing the first
or every chat, a late initial response after a signed-in or signed-out manual
result, login waiting state, and retry after a missing CLI. They also check empty
message/queue/session state. The quick repository gate passes **2,437 Rust tests**
with 36 ignored and **38 site tests**, plus formatting, native all-target/all-feature
Clippy, WASM Clippy, site types and benchmark self-checks. The normal native
optimized-profile compile check also passes. Neither check is a startup benchmark.

The change touches two existing Rust files and adds one Unix-only test module.
The source snapshot pins 1,146 inputs and the same 30 assets. All 49 earlier ledger
entries retain their source pins; 27 now have changed sources, including the
preceding sidebar measurement's full-tree pin. The sidebar implementation is
unchanged by this auth fix, and its native fixture still blocks auth before flight
registration. Its earlier measurements remain archived at their original source
checkpoint. They are not re-pinned to this change.

Receipts and the exact reviewable patch are under
`target/performance-auth-probes-`: `operation-counts.json`, `scope.json`,
`change.patch`, `before-manifest.json`, `after-manifest.json`, the source/assets
archives, `gate-receipt.json`, `release-check-receipt.json` and their logs.
The action ledger adds this as partial Chat constructor evidence. The all-actions
goal remains open, including cold startup with real native frames, mixed workspaces,
provider timing and physical input/presentation.

## Native restoration with controlled auth and native frames

The initial-auth sharing fix now has controlled normal-native measurements through
1, 32 and 128 restored chats. The 128-chat file/constructor-through-native-callback
p50 changes by **-81.2% forward and -81.1% reverse**. These results
apply to the fixture boundary below; total app launch and first-interactive latency
remain unmeasured.

The same optimized native executable selects the retained original behavior
(one independent initial probe per chat) or current sharing (one pending probe for
the synchronous restoration). Actual CLI counters confirm 1/32/128 versus 1/1/1.
This is a reference-behavior comparison within one binary, not a comparison against
a separately built historical executable. Input bytes and complete final saved
models agree across strategies, run orders and measurement families.

### Boundary and results

Each fresh process initializes the normal macOS platform, in-memory app services,
embedded fonts and an empty native window. The fixture waits for window activation
and blank draw opportunities before starting the clock. It then uses the actual
file-loading `Workspace::new` and `Root`, waits for every restored chat's auth status
to be applied on the foreground thread, and finishes at the second following native
callback. That callback follows an intervening draw/present opportunity; it does
not establish GPU completion or physical presentation. The dependency invokes each
callback before its draw, so the second callback provides an opportunity to draw
after status delivery. Auth checks run asynchronously; the active chat can be usable
before every restored chat has received its metadata. The one-chat roughly 25 ms
control includes rendering and callback cadence; it is not a measured readiness
threshold.

There are **720 primary intervals across 744 fresh processes**, with 24 excluded
priming processes. Wall and separately traced CPU/frame families each have 360
primary intervals. Every cell has 30 observations and nearest-rank p95. Forward
uses ascending sizes and original-first adjacent pairs; reverse uses descending
sizes and shared-first adjacent pairs. Each observation is the first workspace
restoration in its process, after the setup above. OS caches are not purged.

The wall boundary runs from file loading/Workspace construction through all auth
foreground delivery and the second following native callback:

| Chats | Order | Original p50 / p95, ms | Shared p50 / p95, ms |
| ---: | --- | ---: | ---: |
| 1 | forward | 25.011 / 26.880 | 24.967 / 26.838 |
| 1 | reverse | 25.010 / 26.554 | 24.994 / 25.576 |
| 32 | forward | 41.489 / 51.054 | 24.984 / 25.578 |
| 32 | reverse | 40.375 / 50.348 | 24.654 / 25.525 |
| 128 | forward | 133.081 / 166.243 | 25.073 / 26.677 |
| 128 | reverse | 133.270 / 175.408 | 25.142 / 26.291 |

The separately instrumented CPU family covers the same interval. Application CPU
includes the app's worker threads and excludes child CLI CPU. Cumulative CPU can
exceed elapsed wall time when multiple app threads run concurrently:

| Chats | Order | Foreground CPU, original → shared p50, ms | Application CPU, original → shared p50, ms |
| ---: | --- | ---: | ---: |
| 1 | forward | 21.966 → 22.111 | 26.447 → 26.637 |
| 1 | reverse | 22.782 → 22.954 | 27.398 → 27.655 |
| 32 | forward | 24.793 → 20.040 | 42.289 → 24.114 |
| 32 | reverse | 23.660 → 19.708 | 40.176 → 24.046 |
| 128 | forward | 34.534 → 21.291 | 205.441 → 25.360 |
| 128 | reverse | 36.244 → 21.832 | 159.428 → 25.704 |

Raw constructor, Root, auth-delivery, callback, CPU and native draw intervals remain
in `comparison4.json`; its chart exposes both orders and p50/p95. Small controls and
tails must be read independently. No CPU-overhead subtraction or conversion from
these intervals to frame rate, INP or first-interactive latency is made.

The data contains pinned general chats with empty messages/queues/sessions. The
final title contains Unicode; visible content therefore differs by case size, while
the original/shared inputs within each size match exactly. The fixture creates its
own non-executable `auth` script in its dedicated temporary working directory. The
existing `/bin/sh` receives the unchanged `auth status` arguments, validates the
status operation, appends a counter line and prints fixed signed-in JSON. Its process,
script and counter-file overhead belongs to these results. Real Claude account-query
latency is not inferred from it.
Meetings, preview scans and update polling remain blocked; credentials, connections
and calendar tokens use in-memory stores. No real account/provider/login action runs.

### Invalid prototype and verification

The first protocol began restoration before confirming window activation. Its
165-process incomplete cohort contains 164 successful runs and one failure of the
end-of-interval active-window guard. The cause of lost focus is not established.
**All 165 prototype runs are excluded**, rather than replacing the failed interval
inside that cohort. Their original receipts/logs remain in `prototype-excluded.json`.
The final protocol checks activity before timing and at the final callback.

A second, complete 744-process cohort used a freshly created executable fake script.
A separate 180-interval Python launch diagnostic found fresh direct execution at
roughly 182–184 ms p50, versus roughly 3–4 ms for a primed reused file or the existing
interpreter in both orders. This identifies a fixture launch characteristic, without
identifying its OS mechanism. That entire native cohort remains separate secondary
evidence in `fresh-executable-secondary.json` and `comparison3.json`. None of its
measurements enter the final interpreter-based cohort above; no diagnostic cost is
subtracted from native timings.

The final fixture uses the existing interpreter and a temporary working directory.
All six debug size/strategy cases preserve the complete saved model from the earlier
protocol, confirming that the directory change did not change persisted content.
Debug timings remain separate from all optimized evidence.

Every final run verifies native fonts, all auth requests/processes/deliveries,
restored thread order and account state, visible sidebar geometry, complete model
state, and acknowledged saved JSON matching that model. Validation and save
acknowledgement occur after the startup clocks. The quick gate passes 2,437 Rust
tests (36 ignored), 38 site tests, formatting, native all-target/all-feature and WASM
Clippy, site types and benchmark self-checks. The normal optimized-profile compile
check passes too. Explicit native configuration guards and separate debug/native
controls are retained alongside these receipts.

The source snapshot pins 1,148 inputs and 30 unchanged assets. Four existing files
and two new fixture modules differ from the preceding checkpoint. Removing the
native-performance-gated additions leaves the normal auth implementation
byte-identical. Existing native action timing functions and setup are preserved.
The binary SHA-256 is `85613d70ce282a7b873b3643addc70f20b405bdd3af057df02cd2f4d27f860cb`; build artifacts confirm normal
native GPUI dependencies without test-support. No dependency, profile, parser,
timeout, persistence, saved-input-generator or sidebar behavior changes are added.

Receipts are under `target/performance-native-auth-`: `candidate4-manifest.json`,
the source/assets archives, `candidate4-build-receipt.json`, `phase34d-plan.json`,
the `phase34d-*` per-process logs/full saved models, `comparison4.json`, `scope4.json`,
`change4.patch`, gate/compile receipts and excluded prototype records. All 50 earlier
ledger entries retain their original measured-source hashes. Current files differ
from those hashes in 28 entries, which remain historical for their affected boundaries
and are not promoted to current-source coverage. The ledger adds this as partial
Chat restoration/auth evidence; the all-actions audit remains open.
Cold storage, rich/mixed/project workspaces, real provider timing, active-chat-first
readiness, total launch and physical input/GPU presentation remain open.

## New-thread initialization without duplicate auth subprocesses

Creating a Claude thread now starts one initial auth query instead of two.
The actual command handler and dispatched GPUI `NewThread` action both show
the reduction. Codex threads retain their one initial Claude-account prefetch.

| Action path | Selected model | Before processes | After processes |
| --- | --- | ---: | ---: |
| `execute_command_action("new-thread")` | Claude Sonnet 4.6 | 2 | 1 |
| `execute_command_action("new-thread")` | GPT-5.4 | 1 | 1 |
| Dispatched `NewThread` | Claude Sonnet 4.6 | 2 | 1 |
| Dispatched `NewThread` | GPT-5.4 | 1 | 1 |

These are actual fake-CLI subprocess counts in GPUI tests, with an in-memory
desktop state and the test platform's rendering. Each case creates a fresh
workspace before installing the explicit fake program. The command case invokes
the handler directly; palette search, physical input and normal native rendering
are outside this boundary. No milliseconds or user-visible latency improvement
are claimed for these actions.

Both revisions produce identical serialized created-thread state after normalizing
only the generated thread ID and update timestamp. The tests also verify the
selected/active thread, model, empty messages and queue, absent session, and
signed-in banner state. A second test starts on Codex, holds a later Claude status
query, and verifies that the prefetched signed-in state stays visible until the
fresh signed-out result arrives. The count test expects two Claude processes on
the baseline and one on the candidate. Created-state and provider-switch checks
apply to both revisions and pass.

**Decision:** The factory passes the selected model into `ChatView` construction
instead of setting it a second time afterward. Restored threads use the same
constructor, replacing their later direct model assignment. Codex prefetch stays
because its result affects the banner during a later switch to Claude. This keeps
one prefetch query even when the selected model is Codex. Public
model changes and manual auth checks retain their independent fresh queries;
initial constructors retain the existing pending-flight sharing policy.

The source diff shows that the constructor body changes only its model
initializer. The public model setter, thread reset, auth implementation, native
fixture, persistence and dependency/assets remain unchanged; auth additions are
test-only fake-program installation and cleanup. Ordinary tests continue to skip
auth processes unless they explicitly install the fixture.

### Verification and limits

The frozen-source quick gate passes 2,439 Rust tests (36 ignored) and 38 site
tests, formatting, native all-target/all-feature and WASM Clippy, site types and
benchmark self-checks. The normal optimized-profile `native-performance` compile
check also passes. These compile checks add no native timing evidence.

The gate also runs the existing saved-thread reference comparisons for metadata,
queues, permissions, provider policy, project paths and legacy timestamps. Those
tests pass with the shared constructor. Restored-thread native timings from the
previous section retain their measured source pins and have not been rerun for
this revision.

The action ledger records this as partial evidence for `desktop:new-thread`.
Its source pins include the new tests. All 51 older entries retain their original
pins and values; 29 now differ from the current source and need affected-boundary
revalidation. The catalogue remains a discovery aid for the broader audit.

Receipts, archived baseline test inputs, full source/assets, the exact patch and
comparison use the filename prefix `target/performance-new-thread-auth-`.
`baseline-contract-manifest.json` reconstructs the baseline inputs after the run
from the production archive plus the archived fake-program installer, cleanup and
two tests. These additions select the fake executable without changing production
auth behavior; the same seam is used on the candidate. Native action latency,
error/concurrency variants, new-thread project/role paths, acknowledged persistence
and remaining semantic actions require separate evidence.

## Thread-catalogue publishing without repeated reconstruction

Workspace redraws now keep the composer's existing thread catalogue when IDs,
titles and order agree. Changed catalogues reuse the existing vector and update
only the differing suffix/fields. Initial publication still constructs the original
owned representation. Every render still compares current values; there is no
revision cache or new invalidation policy.

For 2,048 short-title threads, isolated publishing p50 falls about 80% for
unchanged and last-title updates in both orders. Long-title improvements for
those same two cases range from 17% to 42%. Reversed long-title and absent-global controls remain mixed,
including worse tails; this is not a speedup claim for every variant.

### Isolated publishing comparison

The same optimized GPUI test executable compares the retained original publishing
body with the current method. A source-token check ties the reference to the
pre-change `Workspace::render` block. This is a retained-reference comparison,
not a separately built historical executable. The test platform uses in-memory
services and dummy rendering. The wall clock covers only the publishing call, including
replacement/drop or global update; preparation, complete value checks and global
notification delivery occur outside it. The method's own equality comparisons
are inside the clock; only the test's verification assertions and copied values
are excluded. Layout, normal native frames, physical
input and acknowledged persistence are not measured here.

Two fresh processes run opposite size/method orders through 1, 32 and 2,048
threads, short and long Unicode titles, and four state variants. Each method/case/order
has 10 excluded warmups and 50 primary observations, giving **4,400 primary
intervals and 880 excluded warmups**. Original/current calls alternate in each
case. Reversing one thread is a no-op, so that case is omitted for both one-thread
title shapes. The resulting 22 cases per process give 44 original/current cells
across the two processes. Every call's complete catalogue values equal the retained reference, and
case fingerprints agree across orders. Data and allocation state are prepared warm
before each call. Removing the global does not simulate cold app startup.

Values below are nearest-rank **p50 / p95 microseconds**. Small one-thread
controls approach the timer floor, including zero-duration samples; relative
percentages for those controls are not useful. Full results include all 44 cells.

| Titles / change (2,048 threads) | Original-first run: original | Current | Current-first run: original | Current |
| --- | ---: | ---: | ---: | ---: |
| Short / Unchanged | 104.750 / 122.834 | 20.584 / 26.333 | 106.916 / 121.958 | 20.750 / 24.916 |
| Short / Last title changed | 104.250 / 118.583 | 20.375 / 25.875 | 109.458 / 125.584 | 22.000 / 25.708 |
| Short / Order reversed | 104.166 / 116.750 | 96.792 / 113.333 | 110.791 / 118.292 | 98.667 / 107.458 |
| Short / Global removed | 64.875 / 74.084 | 66.000 / 78.834 | 69.333 / 78.875 | 69.083 / 77.833 |
| Long / Unchanged | 236.625 / 271.208 | 136.125 / 162.375 | 408.625 / 453.500 | 337.000 / 375.375 |
| Long / Last title changed | 234.917 / 268.500 | 135.833 / 161.458 | 405.666 / 437.667 | 336.000 / 375.167 |
| Long / Order reversed | 282.500 / 317.458 | 290.750 / 352.167 | 462.583 / 525.709 | 467.291 / 559.667 |
| Long / Global removed | 199.791 / 241.208 | 190.209 / 233.417 | 358.083 / 449.041 | 384.708 / 460.625 |

### Capacity tradeoff and checks

Reusing standard `Vec`/`String` capacity also retains space after values shrink.
An untimed diagnostic starts with the same 2,048 long-title catalogue, then
shortens every title and removes every thread. Both orders produce these exact
catalogue-owned capacity counts:

| State | Original capacity bytes | Current capacity bytes |
| --- | ---: | ---: |
| All titles shortened to `Short` | 129,986 | 2,494,316 |
| All threads removed | 24 | 98,328 |

These counts include the catalogue struct, vector slots and string capacities;
they exclude allocator overhead and all other app memory. Removed strings are
dropped, while vector capacity remains available. This trades publishing time
against retained memory; whole-app heap and RSS remain unmeasured. Long-lived mixed workspaces
and other mutation positions need further profiling.

Three regressions check exact values after title/ID changes, order and membership
changes, an absent/foreign global, and empty state; unchanged calls preserve buffers,
and a rename within existing capacity reuses storage. The foreign-global test
manually installs an unrelated workspace ID/title pair. The frozen quick gate passes
2,442 Rust tests (37 ignored) and
38 site tests, formatting, native all-target/all-feature
and WASM Clippy, site types and benchmark self-checks. The normal optimized-profile
`native-performance` compile check passes. No normal-native timing is added by it.

The first candidate improved unchanged calls but regressed last-title updates
about 39–42% in the large long-title fixture. Its complete source, binary and both
orders remain under the filename prefix `target/performance-thread-catalog-` as
unaccepted prototype evidence. A parser correction recovered its first JSON row
from the unchanged successful raw log after Rust prefixed it with the test name;
no interval was rerun or excluded. The revised candidate has its own source pins,
binary and runs under `target/performance-thread-catalog2-`.

Receipts include `comparison.json`, `scope.json`, `change.patch`, source/assets
archives, build/gate receipts, raw order logs and capacity diagnostics under that
second prefix. All 52 earlier ledger entries retain their measured pins; 30 now
differ from current source and require affected-boundary revalidation. This entry
is partial Chat/workspace publishing evidence. Recurring publishing costs fall in
both orders for unchanged and last-title cases, and catalogue-value checks pass;
that supports the isolated method change. Absolute long-title timings vary markedly
by run and remain separate. Native action/frame performance and long-lived memory
remain open audit requirements. The all-actions goal remains open.

## Mixed Unicode record queries without unnecessary whole-field casing

The 2026-11-12 local candidate gives the mixed miss `ζ-unseen` about 79% lower latency
in both execution orders, but ASCII control slowdowns remain unresolved. It is
**unmerged and pending regression work**, not accepted as a general read-path
improvement. Its early rejection skips whole-field casing when a necessary ASCII
fragment of a non-ASCII query is absent. Passing the
guard still uses the original whole-string lowercase substring check, preserving
contextual Greek sigma, dotted-I expansion and Kelvin-sign behavior.

The query stores a checked span into its already normalized text. On this
64-bit host, original and current query state both occupy 32 bytes with
8-byte alignment; no additional owned fragment or box is allocated. The public
512-byte query bound fits this span. Larger internal queries that cannot represent
the optional span retain complete original matching. This is an implementation
fallback, not a new input limit or truncation.

Relevant dotted-I or Kelvin sources make an `i`/`k` fragment conservative: those
fields go straight to the original full casing path. Pure Unicode queries also
retain that path. An exact raw fragment hit skips the redundant case-insensitive
fragment scan. The diff leaves record serialization, saved-history inclusion,
paging, permissions, ordering and reply limits untouched. The exercised fixtures
verify reply equality as described below. There is no record index or cache.

### Native bridge comparisons

The baseline is the exact optimized native test executable from the preceding
thread-catalogue checkpoint. The candidate has a separate optimized build. Only
`requests/query.rs` differs among 1,150 pinned sources; all 30 assets agree.
Both binaries use the same fixed production MCP executable and the same expanded
runtime harness, which adds seven Unicode validation cases. Source and runtime
harness pins are recorded separately.

Four fresh GPUI TestPlatform processes run baseline, candidate, candidate,
baseline. Forward/reverse below identify opposite binary execution orders;
dataset and case order remain fixed. These are warm synthetic records with
in-memory app services, not normal native frames or physical input.

Each process checks 10,001 records per domain across Tasks, Calendar, Capture,
Knowledge and Inbox, then replaces Knowledge with Unicode-heavy documents and
two saved versions. The last-record marker lives only in saved history. Each
case has five excluded warmups and 30 primary observations.

The full-stdio clock starts at parent request write/drain and ends at the complete
stdout frame, through the actual MCP process, private Unix socket and foreground
handler. Encoding, parent JSON decoding and assertions are outside it. There are
37 full-stdio cases and 24 direct-handler diagnostics per process: **4,440 full-stdio
and 2,880 handler primary intervals**, with 740 and 480 excluded warmups respectively.
The direct-handler diagnostics use ASCII queries; the target mixed Unicode miss
is measured by the full-stdio boundary.

Values are nearest-rank **p50 / p95 milliseconds**. With 30 samples, p99 is the
maximum; every raw interval and all p99 values remain in `comparison.json`.

| Unicode Knowledge query | Baseline first: baseline | Candidate | Candidate first: baseline | Candidate |
| --- | ---: | ---: | ---: | ---: |
| `lastquartz`, ASCII match only in saved history | 38.436 / 39.277 | 38.780 / 39.386 | 38.404 / 39.444 | 38.551 / 39.079 |
| `no-quartz-match`, ASCII miss | 38.844 / 40.208 | 38.563 / 39.524 | 38.383 / 38.893 | 39.129 / 41.334 |
| `missing-kelvin`, ASCII miss with alias source | 61.846 / 63.569 | 63.469 / 65.497 | 61.585 / 62.996 | 62.284 / 64.172 |
| `ζ-unseen`, absent ASCII fragment | 199.119 / 202.391 | 40.988 / 41.799 | 199.323 / 203.199 | 40.676 / 42.218 |
| `ζ`, pure Unicode miss | 199.275 / 202.025 | 201.709 / 207.521 | 198.903 / 202.171 | 201.357 / 203.831 |
| `Σ`, contextual miss | 199.775 / 202.519 | 200.788 / 204.934 | 199.484 / 201.572 | 202.121 / 204.691 |
| `ζ-missing-kelvin`, ambiguous fragment | 205.787 / 210.087 | 201.069 / 213.290 | 199.495 / 203.213 | 201.910 / 206.728 |
| `ζcontext`, fragment present | 202.700 / 211.768 | 202.695 / 205.577 | 198.747 / 202.579 | 202.496 / 206.024 |
| `ζlinks`, ambiguous fragment present | 199.593 / 202.334 | 201.120 / 203.604 | 199.148 / 202.319 | 202.017 / 203.921 |
| `CAFÉ`, mixed hit | 2.449 / 2.545 | 2.379 / 2.476 | 2.394 / 2.486 | 2.333 / 2.458 |
| `İ K`, expansion hit | 2.443 / 2.527 | 2.337 / 2.455 | 2.355 / 2.549 | 2.357 / 2.468 |
| `ΟΣ Café`, context hit | 2.476 / 2.689 | 2.361 / 2.475 | 2.342 / 2.437 | 2.339 / 2.483 |

The speedup applies to the absent necessary fragment. Pure Unicode, ambiguous
sources and present fragments still require full casing and remain costly.
The ordinary ASCII full-scan controls are included here to expose changes beyond
the target case; early/deep pages and all other cases remain in the full results.

| ASCII full scan | Baseline first: baseline | Candidate | Candidate first: baseline | Candidate |
| --- | ---: | ---: | ---: | ---: |
| Tasks / last record | 18.999 / 19.292 | 18.866 / 19.447 | 19.324 / 19.809 | 19.166 / 19.966 |
| Tasks / miss | 22.164 / 22.738 | 22.093 / 23.301 | 22.603 / 23.616 | 22.201 / 23.018 |
| Calendar / last record | 15.981 / 16.334 | 15.969 / 16.299 | 16.317 / 16.496 | 15.911 / 16.360 |
| Calendar / miss | 15.692 / 17.198 | 15.950 / 16.607 | 16.201 / 16.544 | 16.219 / 16.516 |
| Capture / last record | 13.798 / 14.320 | 13.591 / 14.341 | 13.476 / 14.375 | 14.412 / 15.038 |
| Capture / miss | 13.305 / 13.891 | 13.499 / 14.419 | 13.214 / 13.682 | 14.129 / 14.657 |
| Knowledge / last record | 42.230 / 43.116 | 43.834 / 44.580 | 42.253 / 44.210 | 43.863 / 45.080 |
| Knowledge / miss | 42.446 / 43.263 | 43.762 / 44.765 | 42.551 / 44.159 | 43.822 / 45.833 |
| Inbox / last record | 27.332 / 27.746 | 27.852 / 29.002 | 27.602 / 29.192 | 28.324 / 29.064 |
| Inbox / miss | 27.675 / 28.420 | 27.661 / 28.472 | 27.788 / 29.003 | 27.246 / 32.908 |

These controls and their tails are mixed, so this pass does not establish a
speedup for ASCII queries or every Unicode query. Knowledge full-stdio scans
are roughly 3–4% slower in both orders. Capture full-stdio scans are roughly
7% slower in the candidate-first order; direct-handler Capture controls also
show slowdowns, including about 11% for the baseline-first last-match case.
The following direct-handler controls retain the boundary-specific evidence.

| Direct ASCII handler control | Baseline first: baseline | Candidate | Candidate first: baseline | Candidate |
| --- | ---: | ---: | ---: | ---: |
| Capture / last | 13.317 / 13.502 | 14.796 / 15.392 | 13.504 / 15.329 | 13.449 / 14.808 |
| Capture / none | 13.395 / 13.536 | 14.250 / 14.434 | 13.314 / 13.526 | 13.815 / 15.443 |
| Knowledge / last | 41.878 / 43.819 | 44.386 / 45.858 | 42.156 / 42.877 | 42.381 / 43.384 |
| Knowledge / none | 42.211 / 43.480 | 44.256 / 45.851 | 43.760 / 46.307 | 43.845 / 45.272 |

Resolve these control regressions before merge acceptance. Absolute timings vary
with execution order. Thirty observations within one process do not establish
whether small differences are implementation effects or process/system variation.
Profile these controls and obtain fresh independent process cohorts before
deciding acceptance; this pass sets no numerical acceptance threshold.
No interval was discarded or rerun to replace an unfavorable
timing. Complete parsed reply fingerprints and frame sizes agree across all four
cohorts; the fingerprint canonicalizes outer JSON keys and retains inner record
JSON text. IDs, page counts, next offsets, five-domain permission revocation and
cleanup checks pass.

### Separate finance controls and validation

Eight additional fresh native processes check the shared query's 100,001 ASCII
finance-transaction fixture in the same opposite binary orders: four processes
run handler diagnostics and four run full stdio. Four native
handler cases retain the existing rounded three-decimal p50/p95/p99 summaries
from 50 observations after five warmups; raw handler intervals are unavailable.
Eight warm full-stdio cases use 200 observations after ten warmups, totaling
6,400 primary intervals and 320 excluded warmups. Their full replies, redaction,
paging, catalog mode and error checks agree across cohorts. This table contains
the warm full-stdio finance timings, not the rounded handler diagnostics.

| Finance control | Baseline first: baseline | Candidate | Candidate first: baseline | Candidate |
| --- | ---: | ---: | ---: | ---: |
| finance-first | 0.272 / 0.311 | 0.274 / 0.301 | 0.272 / 0.321 | 0.274 / 0.304 |
| finance-deep | 0.273 / 0.289 | 0.275 / 0.304 | 0.271 / 0.291 | 0.285 / 0.318 |
| finance-last | 0.746 / 0.779 | 0.763 / 0.810 | 0.795 / 0.928 | 0.790 / 0.892 |
| finance-none | 2.038 / 2.166 | 2.014 / 2.174 | 2.047 / 2.215 | 2.041 / 2.215 |

The finance controls are also mixed. The harness separately records 400 fresh
fixed-MCP initialization observations with warm OS caches, after eight excluded
launches. This is an unchanged-transport control, not native app startup evidence.
All controls remain separate from the target query comparison.

All 12 query regressions pass, including the std-reference mixed-text checks,
exhaustive Unicode ASCII-producing mappings, long overlapping fragments and
oversized internal-span fallback. The layout diagnostic confirms sizes only;
it does not prove a causal explanation for timing differences. The quick gate
passes 2,446 Rust tests (37 ignored)
and 38 site tests, plus format, native/WASM Clippy,
site types and benchmark self-checks. The normal optimized native-performance
compile check also passes. An initial gate attempt failed when an unrelated
isolated server test's temporary port was in use. Its failed log is retained;
the exact test passed alone and the full gate rerun passed without source changes.

Two earlier boxed-fragment prototypes remain unaccepted under
`target/performance-unicode-fragment-` and `target/performance-unicode-fragment2-`.
The first added roughly 4–5% to a present-fragment miss; the second reduced that
overhead but regressed ASCII Calendar/Capture scans in both orders. Their query
state was 40 bytes. The compact candidate has independent pins, binary and runs;
the measurements above determine its behavior, not the layout hypothesis alone.

Current receipts, raw logs, source/assets archives, patch, layout diagnostic and
the 148-cell interactive comparison use the filename prefix
`target/performance-unicode-fragment3-`. All 53 older ledger entries retain their
original pins; 39 now differ from current source and need affected-boundary
revalidation. The new entry is partial candidate Knowledge-read variant evidence.
This is a verified stopping point with no benchmark/build workers left running;
the code remains local and uncommitted. Control regression work comes next. Cold
hydration, normal native frames, physical input, concurrent requests, other data
distributions and remaining actions still need measurement. The all-actions goal
remains open.

The interactive comparison has 74 generic full-stdio cells (37 cases in two
orders), 48 generic handler cells, 18 finance full-stdio/control cells and eight
rounded finance handler cells. Each cell compares baseline and candidate;
the initialization controls are included among the finance full-stdio/control
cells. The raw files retain the four individual cohorts.

## Shared record-query checks without redundant field scans

The current shared-query candidate remains unaccepted. The mixed Unicode miss has 78.8–79.9% lower full-stdio p50 across four comparisons, but Finance and projected-read controls still regress. The explicit-inlining experiment was rejected and removed; current source is byte-identical to the already measured sixth candidate. No merge or push was made.

The 2026-11-12 current candidate derives its ASCII-source selector once per query,
then selects a matching path before each field list. A normalized ASCII query
without `i` or `k` uses the ASCII matcher without field classification. Those are the only
ASCII bytes that lowercasing a non-ASCII scalar can produce, guarded by the
exhaustive std-mapping test. Classifying the field cannot change the result for
other ASCII needles. Queries containing `i` or `k`, and all non-ASCII queries,
keep their classification path. Alias queries retain the original source checks
and normalization block, without the fifth candidate's extra per-field guard.

The earlier necessary-fragment guard remains: a non-ASCII query can reject a
field whose required ASCII fragment is absent, then otherwise uses the original
whole-string lowercase substring check. Relevant dotted-I/Kelvin sources defer
to std. Pure Unicode and present-fragment cases still require full casing.

An ordered query layout keeps the original ASCII flag offsets (24–26), with a
checked borrowed fragment span in the remaining storage. Original and current
state are 32 bytes with 8-byte alignment on this host. The new immutable selector
occupies the existing padding byte at offset 27. No extra fragment string
or box is allocated. Unrepresentable internal spans retain full original
matching; public query/offset limits are unchanged.

### What is compared

The baseline is the exact pre-fragment optimized native test executable from the
thread-catalogue checkpoint. The current candidate has a separate optimized
build. Only `requests/query.rs` differs among 1,150 pinned sources; all 30 assets
agree. The final revision adds the selector and selects the path before field
iteration. This original-baseline comparison establishes combined behavior;
it does not isolate the causal contribution of each intermediate change.

Both binaries use the same fixed production MCP executable and runtime scripts.
The original seven added Unicode validation cases remain unchanged. Projected
reads have a separate supplemental runtime manifest. Together, compiled and
runtime pins cover 1,152 unique source/script files. Source/assets archives,
build receipts and the incremental patch are retained.

The generic and projected families each run eight fresh native processes, in
baseline/current/current/baseline order, then repeat that sequence with fresh
processes. Finance uses that sequence separately for its handler and stdio
boundaries. Tables retain four
comparisons separately. Dataset/case order is fixed. These are warm synthetic
in-memory services on GPUI TestPlatform. Normal native frames, physical input,
cold hydration and providers are outside the measurements.

The handler clock covers actual handler entry through returned value. Full stdio
covers parent write/drain through the complete stdout frame, including the real
MCP process, private Unix socket and foreground native handler. Request encoding,
parent JSON decoding, assertions and fixture construction are outside the clock.
Four Cargo profile locks exclude builders; no GUI/profiler observes primary runs.

| Family / boundary | Primary observations | Excluded warmups |
| --- | ---: | ---: |
| Generic handler: 24 ASCII-query cases per cohort | 5,760 | 960 |
| Generic full stdio: 37 cases per cohort | 8,880 | 1,480 |
| Projected handler: 126 cases per cohort | 30,240 | 5,040 |
| Projected full stdio: 126 cases per cohort | 30,240 | 5,040 |
| Separate Finance full stdio: eight warm cases per cohort | 12,800 | 640 |

Generic/projected cases use 30 observations after five warmups. Finance warm stdio
uses 200 after ten. Finance handler diagnostics retain 1,600 observations as
existing three-decimal summaries (160 warmups); raw handler intervals are unavailable.
Another 800 fixed-MCP initialization observations follow 16 excluded launches.
Those warm-OS-cache transport controls are separate from native app startup.
There are 32 native processes total: eight generic, eight projected and 16 Finance
(eight handler, eight stdio). Boundaries and initialization controls are not pooled.

### Target query and affected read controls

Generic fixtures contain 10,001 records per domain across Tasks, Calendar,
Capture, Knowledge and Inbox. Unicode-heavy Knowledge has two complete saved
versions; the last-match marker occurs only in saved history. The target mixed
miss `ζ-unseen` is measured by full stdio, not the ASCII-only direct-handler cases.
This table gives nearest-rank **p50 / p95 milliseconds**.

| Execution order / cohort | Baseline p50 / p95 | Current p50 / p95 | p50 change |
| --- | ---: | ---: | ---: |
| Baseline first / 1 | 197.311 / 206.401 | 41.914 / 43.416 | -78.8% |
| Candidate first / 1 | 196.957 / 205.488 | 39.659 / 41.178 | -79.9% |
| Baseline first / 2 | 196.111 / 204.005 | 39.958 / 41.137 | -79.6% |
| Candidate first / 2 | 196.248 / 204.322 | 41.111 / 42.434 | -79.1% |

The following generic, projected and Health tables show the **full-stdio
boundary**, with **baseline → current p50 milliseconds**. Full p95/p99 and
all retained raw intervals remain in the comparisons and interactive view;
Finance handler diagnostics are available only as rounded summaries. For the
30-observation cases, nearest-rank p99 is the maximum. The 1,304 interactive cells
include all boundaries: 148 generic stdio, 96 generic handler, 36 Finance
stdio/initialization, 16 rounded Finance handler, and 504 each projected handler
and stdio. Every cell compares baseline/current for one execution order/cohort.

| Read / case | Baseline first / 1 | Candidate first / 1 | Baseline first / 2 | Candidate first / 2 |
| --- | ---: | ---: | ---: | ---: |
| Tasks / last | 19.272 → 18.775 | 19.777 → 18.372 | 19.044 → 19.235 | 18.299 → 18.183 |
| Tasks / none | 22.316 → 21.571 | 22.312 → 21.476 | 22.477 → 23.185 | 22.292 → 21.680 |
| Calendar / last | 16.646 → 14.703 | 15.571 → 14.791 | 16.328 → 15.202 | 16.575 → 15.503 |
| Calendar / none | 16.572 → 14.808 | 15.628 → 14.771 | 16.396 → 14.665 | 16.329 → 15.375 |
| Capture / last | 13.592 → 12.597 | 13.225 → 12.405 | 13.671 → 14.024 | 13.373 → 12.547 |
| Capture / none | 13.542 → 12.720 | 13.318 → 12.424 | 13.481 → 13.993 | 13.445 → 12.457 |
| Knowledge / last | 41.718 → 40.452 | 41.868 → 39.465 | 41.441 → 40.545 | 42.142 → 39.742 |
| Knowledge / none | 43.521 → 41.324 | 43.235 → 41.621 | 42.757 → 40.866 | 42.180 → 39.807 |
| Inbox / last | 27.765 → 25.758 | 27.163 → 26.290 | 27.221 → 28.199 | 28.181 → 26.942 |
| Inbox / none | 27.693 → 25.790 | 27.195 → 26.271 | 27.338 → 27.283 | 27.595 → 26.264 |

Projected fixtures cover 18 target/privacy variants with 100,001 records each,
except Chat's 10,001 threads. Each checks first/deep pages, last match, miss,
filtered deep/final pages and excluded-body misses. The selected miss rows below
span all 18 variants; they do not replace the other six cases in the full results.

| Read / case | Baseline first / 1 | Candidate first / 1 | Baseline first / 2 | Candidate first / 2 |
| --- | ---: | ---: | ---: | ---: |
| chat / none | 0.397 → 0.372 | 0.406 → 0.391 | 0.407 → 0.412 | 0.391 → 0.471 |
| finance/accounts/exact / none | 1.110 → 0.941 | 1.265 → 0.897 | 1.240 → 0.920 | 1.248 → 0.969 |
| finance/accounts/redacted / none | 1.193 → 0.930 | 1.151 → 1.021 | 1.303 → 1.037 | 1.130 → 0.958 |
| finance/budgets/exact / none | 1.309 → 0.964 | 1.177 → 0.859 | 1.223 → 0.984 | 1.142 → 0.926 |
| finance/budgets/redacted / none | 1.364 → 0.979 | 1.208 → 0.881 | 1.231 → 1.004 | 1.164 → 0.919 |
| finance/holdings/exact / none | 1.534 → 1.098 | 1.384 → 1.099 | 1.520 → 1.117 | 1.373 → 1.135 |
| finance/holdings/redacted / none | 1.343 → 1.095 | 1.383 → 1.101 | 1.422 → 1.186 | 1.403 → 1.131 |
| finance/income_streams/exact / none | 1.401 → 1.019 | 1.210 → 0.997 | 1.233 → 1.008 | 1.384 → 0.978 |
| finance/income_streams/redacted / none | 1.224 → 1.027 | 1.210 → 1.061 | 1.216 → 1.023 | 1.397 → 0.967 |
| finance/transactions/exact / none | 1.528 → 1.409 | 1.752 → 1.356 | 1.494 → 1.316 | 1.526 → 1.334 |
| finance/transactions/redacted / none | 1.488 → 1.400 | 1.829 → 1.320 | 1.467 → 1.303 | 1.555 → 1.303 |
| health/habits / none | 1.081 → 0.954 | 1.007 → 0.819 | 1.028 → 0.844 | 1.002 → 0.797 |
| health/recovery / none | 0.626 → 0.622 | 0.584 → 0.606 | 0.585 → 0.555 | 0.578 → 0.561 |
| health/sleep / none | 0.568 → 0.552 | 0.621 → 0.617 | 0.624 → 0.557 | 0.595 → 0.556 |
| health/workouts / none | 1.351 → 0.994 | 1.172 → 0.983 | 1.107 → 0.995 | 1.251 → 0.936 |
| meeting-notes / none | 1.027 → 0.769 | 1.000 → 0.776 | 0.998 → 0.814 | 1.013 → 0.810 |
| meetings / none | 1.316 → 1.542 | 1.445 → 1.250 | 1.321 → 1.156 | 1.513 → 1.229 |
| people / none | 1.166 → 1.056 | 1.302 → 1.084 | 1.389 → 1.160 | 1.242 → 1.079 |

Health Sleep/Recovery last and filtered-final scans are shown explicitly because
the ordered-only prototype regressed them substantially in several cohorts.

| Read / case | Baseline first / 1 | Candidate first / 1 | Baseline first / 2 | Candidate first / 2 |
| --- | ---: | ---: | ---: | ---: |
| health/sleep / last | 0.812 → 0.765 | 0.914 → 0.883 | 0.907 → 0.761 | 0.853 → 0.793 |
| health/sleep / filtered-final | 1.252 → 1.277 | 1.408 → 1.348 | 1.327 → 1.244 | 1.399 → 1.305 |
| health/recovery / last | 0.910 → 0.840 | 0.844 → 0.882 | 0.898 → 0.785 | 0.840 → 0.793 |
| health/recovery / filtered-final | 1.312 → 1.339 | 1.293 → 1.396 | 1.333 → 1.228 | 1.328 → 1.286 |

The separate Finance `missing` query contains `i`, so it retains the original
field classification and alias checks. Its handler summaries and full-stdio
measurements remain separate.

| Read / case | Baseline first / 1 | Candidate first / 1 | Baseline first / 2 | Candidate first / 2 |
| --- | ---: | ---: | ---: | ---: |
| Finance handler / `missing` | 1.851 → 1.917 | 1.830 → 2.006 | 1.837 → 1.997 | 1.835 → 1.893 |
| Finance stdio / `missing` | 1.924 → 1.989 | 1.933 → 2.059 | 2.046 → 1.979 | 2.017 → 2.046 |

Finance direct-handler `missing` p50 is 3.2–9.6% higher in all four comparisons. Its full-stdio p50 ranges from 3.3% lower to 6.5% higher. Ten projected full-stdio cases have higher p50 in all four comparisons, including hidden-content misses in Budgets, Holdings, Income Streams, Sleep and Meeting Notes. Sleep hidden-miss p50 increases 5.2–10.0%. These controls prevent acceptance; faster plain-ASCII scans do not cancel them. Per-case p95/p99 and every observed regression remain available.

Thirty requests within one process are not 30 independent process replications.
Small differences and tails remain observations, not a statistical proof of
equivalence. All primary observations were retained in their original form:
raw arrays where captured, rounded summaries for the Finance handler. Every generic and projected
complete parsed reply fingerprint and frame size agrees across all eight cohorts;
Finance's eight warm replies also agree. Fingerprints canonicalize outer JSON
keys while preserving inner record JSON text. Fixture assertions check IDs,
counts, next offsets, projection/redaction, excluded content and grant revocation.

A later acceptance decision must remeasure the slower controls at handler return
and through full stdio, in both binary orders with independent cohorts. Recurring
slowdowns keep the candidate unaccepted. Full-stdio latency is the action-facing
measure; handler results diagnose native work. Review p50 and the retained
p95/p99 separately. These runs establish no numerical noise tolerance or
statistical equivalence for small differences.

### Scope, checks and remaining work

All 12 query regressions pass against std lowercase-substring semantics, including
Unicode contexts/expansions, mixed/generated fields, overlap and oversized-span
fallback. The frozen-source quick gate passes 2,446 Rust
tests (37 ignored) and 38 site tests,
plus format, native all-target/all-feature and WASM Clippy, site types and benchmark
self-checks. The normal optimized native-performance compile check also passes.
No new record index, cache, input/page/reply limit or permission rule is introduced.

All earlier prototypes remain separate evidence. The first two used boxed
fragments. The compact third candidate exposed ASCII control costs. The ordered
fourth candidate restored flag offsets but retained Knowledge handler slowdowns
and exposed large Health Sleep/Recovery increases in several projected cohorts.
Its 32 native processes and all unfavorable observations are retained. The fifth
candidate skipped classification through a per-field condition, improving many
plain-ASCII scans but slowing Finance's `missing` control in all four comparisons.
It also retains its complete 32-process bundle. A seventh experiment explicitly
inlined the field-list dispatcher. Finance's miss p50 was higher in all four
comparisons at both boundaries (6.9–12.7% at handler return and 9.8–18.3%
through stdio). That experiment also retains all 32 native processes; its inline
hint was removed. Separate baseline comparisons do not isolate the effect
of inlining relative to the sixth candidate.

A separate isolated query-method probe compared the original body, that guard,
a cached predicate, direct Unicode-source checks, an enum dispatch, a Boolean
selector and explicit field-list inlining. Direct checks regressed short fields; caching the predicate alone did
not remove the field-loop overhead. Selecting the path before the field list
was chosen for native validation. All four probe rounds remain separate from
action measurements. Each uses 30 batch intervals of 200 calls after five warmup
intervals, with opposite case/method orders in two processes. Layout and isolated
probes alone never establish native timing acceptance or causality.

Separate observer-affected profiles of the rejected seventh experiment place
matching work in a Finance predicate closure after the direct `matches_fields`
calls disappear. This identifies code placement, not the cause of the slower
controls. The dedicated transaction fixture is ASCII; the projected fixture
uses Unicode-heavy labels. Next, inspect the outlined predicate and scan-loop
code generation before choosing another optimization. Profiling observations
are excluded from primary timing evidence.

The action inventory now contains **31 public MCP tools**. Earlier counts of 32
included `bruhs_read`, an internal argument-parser diagnostic label with no tool
registration. Discovery now reads tool declarations. A separate check against
the fixed MCP executable verifies 28 bridge tools; source declarations add the
three standalone-only tools. This catalogue check invokes no workspace data
requests and makes no latency claim. No ledger entry referred to the false tool.

Current artifacts use the filename prefix `target/performance-unicode-fragment6-`.
Local evidence links:

- [Generic comparisons](../target/performance-unicode-fragment6-comparison.json), [Finance comparisons](../target/performance-unicode-fragment6-finance-comparison.json) and [projected comparisons](../target/performance-unicode-fragment6-projected-comparison.json) retain every case.
- [Interactive comparison](../target/performance-unicode-fragment6-comparison.html) selects all 1,304 cells.
- [Current-source verification](../target/performance-unicode-fragment6-restored-native-verification.json) binds the frozen build and observations.
- [Rejected inline experiment](../target/performance-unicode-fragment7-prototype-status.json) and [diagnostic observation](../target/performance-unicode-fragment7-diagnostic-observation.json) retain the unfavorable result and profiling limits.
- [Gate receipt](../target/performance-unicode-fragment6-gate-receipt.json) and [catalogue verification](../target/performance-unicode-fragment6-inventory-runtime-catalogue.json) record checks.

The single shared comparison bundle maps partial current variants to 11 existing
MCP tool identities; these are not 11 independent experiments or whole-action
acceptances. All 54 older ledger entries retain their measured pins, with 40 now
differing from current source. The audit remains incomplete across semantic action
variants, normal native/browser rendering, cold storage, concurrency and providers.
Pure Unicode/present/ambiguous queries and complete JSON serialization remain
read-path work. Code remains local and uncommitted.

## Outlined overlap fallback in native record reads

Outlining the overlap fallback removes verified dispatcher register overhead, but the combined current candidate remains unaccepted. The target mixed Unicode miss has 78.2–80.0% lower full-stdio p50 across four comparisons. Dedicated Finance miss results are now mixed; repeated Health, projected-read and Unicode control slowdowns still block acceptance. No merge or push was made.

The eighth candidate changes only the placement of the existing overlapping-window
fallback relative to the sixth candidate. Ordinary matching still probes candidate
windows with the same ASCII case-insensitive comparison. An overlapping failed
window still lowercases and searches the same remaining suffix. That fallback now
has its own non-inlined helper. Query construction, dispatch, Unicode guards,
normalization, layout and all 12 query regressions are unchanged.

### Why this change was tested

In the sixth candidate's compiled field dispatcher, four callee-saved vector
registers were stored in every prologue. Three vector constants were prepared
on either ASCII branch but used only by the overlapping-window lowercase
fallback. Its stack frame was 208 bytes; the original baseline used 160 bytes.
The rejected seventh candidate had added an outlined iterator call inside the
Finance predicate, so forcing dispatcher inlining was removed.

The eighth candidate's disassembly confirms that the field dispatcher calls the
separate overlap helper and has no callee-saved vector stores. This establishes
code placement and register state, not a causal latency proof by itself.

A separate isolated probe compares the original, sixth and eighth bodies in one
optimized binary. All 38 cases check results against std lowercase-substring
semantics. It retains two opposite case/method orders, each with 30 batch
intervals of 200 calls after five warmup intervals. It includes real transaction
field shapes, single Health IDs and long overlapping-prefix cases. Constructors,
field preparation and reference assertions stay outside the clock. This probe
supports candidate selection; the native comparisons below determine action
evidence.

### Native boundaries and controls

The baseline remains the exact pre-fragment optimized native executable. Only
`requests/query.rs` differs among 1,150 pinned sources; all 30 assets agree.
Compiled and supplemental runtime manifests cover 1,152 unique source/script
pins. Both native test executables host the actual Workspace handlers. Each launches
the same separate production `bruhs-mcp` executable over stdio and connects it
to that native backend through a private Unix bridge. Runtime scripts also agree. These comparisons measure the combined current
implementation against the original baseline, rather than isolating this helper's
contribution from the sixth candidate.

Each native fixture family has two independent comparison replicates per binary
order. Each replicate pairs one baseline process with one candidate process.
The 32 native processes across all families comprise eight generic, eight projected, eight dedicated Finance handler and eight
dedicated Finance stdio processes. Dataset and case order stay fixed. Each primary
run holds four Cargo profile locks, with builders and profilers excluded. Warm
in-memory fixtures use GPUI TestPlatform, with no real account/provider work.

Handler timing covers actual handler entry through returned value. Full stdio
covers the parent write/drain through the complete stdout response frame, including
the MCP process, Unix socket and foreground native handler. Fixture creation,
request encoding, parent JSON decoding and assertions stay outside that clock.

| Family / boundary | Primary observations | Excluded warmups |
| --- | ---: | ---: |
| Generic handler | 5,760 | 960 |
| Generic full stdio | 8,880 | 1,480 |
| Projected handler | 30,240 | 5,040 |
| Projected full stdio | 30,240 | 5,040 |
| Dedicated Finance full stdio | 12,800 | 640 |

Generic/projected cases use 30 observations after five warmups. Finance full
stdio uses 200 after ten. Another 1,600 Finance handler observations remain as
three-decimal summaries after 160 warmups; their raw intervals were not captured.
The 800 fixed-MCP initialization observations follow 16 excluded launches and
remain transport controls, separate from native app startup.

The following tables give **baseline → current p50 milliseconds**, with boundaries
named explicitly. Generic fixtures have 10,001 complete records per domain;
Unicode-heavy Knowledge includes two complete saved versions. Projected fixtures
use Unicode-heavy labels and 100,001 records per target/privacy variant, except
Chat's 10,001 threads. The dedicated transaction fixture is ASCII.

| Case / boundary | Baseline first / 1 | Candidate first / 1 | Baseline first / 2 | Candidate first / 2 |
| --- | ---: | ---: | ---: | ---: |
| Knowledge `ζ-unseen` / full stdio | 197.400 → 39.499 | 196.902 → 40.952 | 196.069 → 42.652 | 197.186 → 39.599 |

Finance `missing` and projected `hidden-perf-body` contain `i` and exercise the
ASCII-source checking path. Sleep/Recovery long scans and hidden misses remain
explicit controls from the earlier rejected candidates.

| Case / boundary | Baseline first / 1 | Candidate first / 1 | Baseline first / 2 | Candidate first / 2 |
| --- | ---: | ---: | ---: | ---: |
| Finance `missing` / handler | 1.874 → 1.937 | 1.948 → 1.908 | 1.833 → 1.845 | 1.840 → 1.847 |
| Finance `missing` / stdio | 1.991 → 2.020 | 2.072 → 2.142 | 2.006 → 2.075 | 2.106 → 1.944 |
| health/sleep / last / full stdio | 0.809 → 0.754 | 0.846 → 0.757 | 0.913 → 0.771 | 0.837 → 0.744 |
| health/sleep / filtered-final / full stdio | 1.377 → 1.247 | 1.429 → 1.237 | 1.403 → 1.321 | 1.390 → 1.253 |
| health/sleep / hidden-miss / full stdio | 0.583 → 0.595 | 0.593 → 0.599 | 0.625 → 0.603 | 0.551 → 0.594 |
| health/recovery / last / full stdio | 0.844 → 0.754 | 0.887 → 0.763 | 0.932 → 0.772 | 0.859 → 0.750 |
| health/recovery / filtered-final / full stdio | 1.385 → 1.254 | 1.451 → 1.257 | 1.418 → 1.266 | 1.335 → 1.279 |
| health/recovery / hidden-miss / full stdio | 0.558 → 0.601 | 0.621 → 0.600 | 0.611 → 0.600 | 0.561 → 0.597 |

All 18 projected target/privacy variants retain their excluded-content miss:

| Case / boundary | Baseline first / 1 | Candidate first / 1 | Baseline first / 2 | Candidate first / 2 |
| --- | ---: | ---: | ---: | ---: |
| chat / hidden miss / full stdio | 1.151 → 1.157 | 1.150 → 1.112 | 1.145 → 1.112 | 1.267 → 1.160 |
| finance/accounts/exact / hidden miss / full stdio | 6.632 → 6.608 | 6.429 → 7.113 | 6.852 → 7.373 | 6.717 → 6.779 |
| finance/accounts/redacted / hidden miss / full stdio | 6.617 → 6.923 | 6.549 → 7.133 | 6.532 → 7.149 | 6.809 → 6.769 |
| finance/budgets/exact / hidden miss / full stdio | 6.833 → 6.738 | 6.615 → 6.919 | 6.677 → 6.851 | 6.506 → 7.090 |
| finance/budgets/redacted / hidden miss / full stdio | 6.939 → 6.961 | 6.975 → 7.202 | 6.507 → 6.666 | 6.493 → 6.989 |
| finance/holdings/exact / hidden miss / full stdio | 6.620 → 6.933 | 6.772 → 6.836 | 6.590 → 6.850 | 6.858 → 6.973 |
| finance/holdings/redacted / hidden miss / full stdio | 7.243 → 6.796 | 6.610 → 6.849 | 6.560 → 7.008 | 6.778 → 7.254 |
| finance/income_streams/exact / hidden miss / full stdio | 6.582 → 6.677 | 6.502 → 6.660 | 6.548 → 6.715 | 6.509 → 6.700 |
| finance/income_streams/redacted / hidden miss / full stdio | 7.103 → 6.681 | 6.977 → 6.647 | 6.493 → 6.743 | 6.453 → 6.599 |
| finance/transactions/exact / hidden miss / full stdio | 6.601 → 6.730 | 6.949 → 6.683 | 6.603 → 6.701 | 6.532 → 6.884 |
| finance/transactions/redacted / hidden miss / full stdio | 6.685 → 6.664 | 6.906 → 6.663 | 6.637 → 6.590 | 6.494 → 6.640 |
| health/habits / hidden miss / full stdio | 6.573 → 6.613 | 6.510 → 7.070 | 6.567 → 6.932 | 6.471 → 6.724 |
| health/recovery / hidden miss / full stdio | 0.558 → 0.601 | 0.621 → 0.600 | 0.611 → 0.600 | 0.561 → 0.597 |
| health/sleep / hidden miss / full stdio | 0.583 → 0.595 | 0.593 → 0.599 | 0.625 → 0.603 | 0.551 → 0.594 |
| health/workouts / hidden miss / full stdio | 6.513 → 6.464 | 6.403 → 6.596 | 6.754 → 6.790 | 6.488 → 6.943 |
| meeting-notes / hidden miss / full stdio | 6.497 → 6.536 | 6.441 → 6.609 | 6.365 → 6.642 | 6.612 → 6.658 |
| meetings / hidden miss / full stdio | 7.040 → 7.095 | 6.869 → 7.096 | 6.852 → 6.921 | 7.012 → 6.953 |
| people / hidden miss / full stdio | 6.649 → 6.875 | 6.794 → 6.620 | 6.556 → 6.924 | 6.724 → 6.845 |

Dedicated Finance `missing` handler p50 ranges from 2.1% lower to 3.4% higher; full-stdio p50 ranges from 7.7% lower to 3.5% higher. Dedicated Finance `missing` full-stdio p95 is 10.3% higher in the baseline-first repeat and remains recorded. Sleep hidden-miss handler p50 is 6.8–11.1% higher in all four comparisons; Recovery is 4.1–12.3% higher. Fifteen projected full-stdio cases have higher p50 in all four comparisons. Budget exact filtered-final p50 increases 2.7–22.2%, and its redacted counterpart increases 4.7–12.7%. Five Unicode-dataset full-stdio controls also have higher p50 in all four comparisons; these include the ambiguous ASCII miss and full-casing fallback shapes. Full per-case results and tails remain separate. These findings prevent general read-path acceptance.

The fifteen recurring projected full-stdio increases comprise five hidden-miss
rows above and ten additional page/query shapes:

| Case / boundary | Baseline first / 1 | Candidate first / 1 | Baseline first / 2 | Candidate first / 2 |
| --- | ---: | ---: | ---: | ---: |
| finance/accounts/exact/deep / full stdio | 0.308 → 0.314 | 0.305 → 0.310 | 0.307 → 0.323 | 0.308 → 0.323 |
| finance/budgets/exact/filtered-deep / full stdio | 2.544 → 2.701 | 2.782 → 2.896 | 2.569 → 2.778 | 2.568 → 2.713 |
| finance/budgets/exact/filtered-final / full stdio | 2.381 → 2.554 | 2.433 → 2.972 | 2.421 → 2.487 | 2.402 → 2.632 |
| finance/budgets/redacted/first / full stdio | 0.267 → 0.299 | 0.269 → 0.293 | 0.270 → 0.285 | 0.265 → 0.271 |
| finance/budgets/redacted/deep / full stdio | 0.263 → 0.282 | 0.263 → 0.289 | 0.265 → 0.282 | 0.266 → 0.269 |
| finance/budgets/redacted/filtered-deep / full stdio | 2.506 → 2.768 | 2.611 → 2.753 | 2.592 → 2.717 | 2.579 → 2.681 |
| finance/budgets/redacted/filtered-final / full stdio | 2.388 → 2.691 | 2.493 → 2.650 | 2.421 → 2.535 | 2.423 → 2.561 |
| health/habits/first / full stdio | 0.262 → 0.271 | 0.262 → 0.267 | 0.264 → 0.269 | 0.265 → 0.273 |
| health/habits/deep / full stdio | 0.259 → 0.259 | 0.256 → 0.263 | 0.262 → 0.272 | 0.261 → 0.270 |
| health/sleep/first / full stdio | 0.239 → 0.243 | 0.238 → 0.243 | 0.236 → 0.246 | 0.240 → 0.245 |

Sleep/Recovery match only the ID, yet their hidden-miss handler controls still
regress. Their adapters pass one-element field arrays through the general query
dispatcher. Both native binaries retain four dispatcher callsites in the Health
reader. This favors isolating direct single-field matching and moving query-mode
selection outside repeated record work. These are hypotheses for the next
record-scan probe, not established causes.

All 1,304 boundary/case/order cells remain in the comparisons and interactive
view. Nearest-rank p95/p99 stay separate from p50; for 30 samples, p99 is the
maximum. All observations remain in the form originally captured, with raw
arrays except the rounded dedicated Finance handler summaries. Requests within
one process are not independent process replications. Small differences are not
a statistical proof of equivalence, and no numerical noise tolerance is asserted.

Complete parsed reply fingerprints and frame sizes agree across the eight
process runs (four baseline, four candidate) of each boundary family. Outer JSON keys are canonicalized; inner record JSON text remains
intact. Fixture checks cover IDs, counts, offsets, saved-history matches,
projection/redaction, excluded content and grant revocation. All unfavorable
intermediate experiments remain available.

### Checks and remaining scope

The frozen-source quick gate passes 2,446 Rust tests (37 ignored) and 38 site
tests, including all 12 std-reference query regressions. Format, native
all-target/all-feature and WASM Clippy, site types and benchmark self-checks pass.
The normal optimized native-performance compile check also passes.

No query/page/reply limit, permission rule, record index or cache is introduced.
Normal native frames, physical input, browser paint, cold hydration, concurrency
and provider latency remain outside these measurements. Pure Unicode,
ambiguous/present-fragment queries and complete JSON serialization remain costs.
Code is local and uncommitted; the full action-performance goal remains open.

Local evidence:

- [Generic comparisons](../target/performance-unicode-fragment8-comparison.json), [Finance comparisons](../target/performance-unicode-fragment8-finance-comparison.json) and [projected comparisons](../target/performance-unicode-fragment8-projected-comparison.json).
- [Native evidence verification](../target/performance-unicode-fragment8-native-verification.json) and [gate receipt](../target/performance-unicode-fragment8-gate-receipt.json).
- [Dispatcher assembly](../target/performance-unicode-fragment8-assembly-verification.json), [Health callsites](../target/performance-unicode-fragment8-health-assembly.json), [isolated probe](../target/performance-unicode-fallback-probe-comparison.json) and [incremental patch](../target/performance-unicode-fragment8-incremental-change.patch).
- [Interactive comparison](../target/performance-unicode-fragment8-comparison.html), with every affected case selectable.

This is one shared experiment bundle mapped to 11 existing MCP tool identities,
not 11 independent experiments or whole-action acceptances. All 65 earlier ledger
entries retain their original pins; 51 now differ from current source.

## Direct single-field matching in Health reads

The two direct Health predicates are retained because they use the existing one-field operation, preserve reference results, and are supported by two isolated diagnostic rounds. In the combined native comparison against the original baseline, all five measured nonempty cases per section (last match, miss, filtered deep, filtered final and excluded-content miss) have lower p50 for both Sleep and Recovery: 9.5–26.6% at handler return and 4.1–24.4% through full stdio. Those native gains cannot be assigned solely to the two edits. Favorable and unfavorable median patterns both require causal caution; broader performance acceptance remains open.

Sleep and Recovery search only their public ID. They now call the existing
single-field matcher directly, replacing a one-element field list. Relative to
the eighth checkpoint, these are the only two source edits. Query construction,
Unicode matching, normalization, projections, paging and permissions are unchanged.

### Candidate selection and its limits

A standalone diagnostic uses the actual Finance/Health model types and production
page encoder. It covers 94 query/page cases across an ASCII transaction ledger,
Unicode transaction and budget exact/redacted variants, Sleep and Recovery.
Each implementation must return the complete standard-library reference page.
The chosen direct predicate improves the single-ID controls in both execution
orders of two probe rounds. The general mode iterator, fixed-size field arrays,
ASCII-first ordering and combined variants had repeat higher medians in other
cases and were not selected for native validation. These selection observations
do not establish that those designs are inherently slower.

| Isolated study | Methods | Primary scan observations | Excluded warmup scans |
| --- | ---: | ---: | ---: |
| Record-scan round 2 | 4 | 22,560 | 3,760 |
| Record-scan round 3 | 7 | 39,480 | 6,580 |

Each round has one optimized binary and two processes with opposite target/case/
method order. Each case uses 30 observations after five warmups. The two rounds
use different binaries; compare methods within a round. Data construction,
reference computation, assertions and Value destruction remain outside timing.
Query construction, matching, paging, projection, encoding and decoding through
returned Value are inside. Root-lock dependency versions are pinned. These are
read-core diagnostics, excluding native authorization, GPUI and MCP transport.
They do not establish whole-action acceptance or reproduce every native regression.

The first iterator prototype failed its projection-count contract before any
SCAN observations: mapping before skipping visited 101 records instead of the
required 51 for a 50-record page and lookahead. Its failed receipt is retained.
The corrected prototype keeps skipping before projection and passes offset,
continuation, empty-field bypass and projection-count checks. That broader iterator
is not adopted. The three added query variants also pass all 36 std-reference
tests before timing.

### Native comparison

The baseline is the same pre-fragment optimized executable used in earlier
checkpoints. Compared with that baseline, the current implementation includes the
earlier query optimizations and the two Health caller changes. The matrix measures
combined behavior, rather than isolating this caller change from checkpoint eight.
Only `requests/query.rs` and `requests/health_read.rs` differ among 1,150 pinned
source/script files; all 30 assets agree. The runtime union adds two unique
scripts, giving 1,152 source/script pins. Assets are counted separately, for
1,182 unique source/script/asset files in total.

Both native test executables host actual Workspace handlers. Each launches the
same separate production `bruhs-mcp` process over stdio and connects it through a
private Unix bridge. The 32 native processes comprise eight generic, eight
projected, eight dedicated Finance handler and eight dedicated Finance stdio runs.
Each family has two separate baseline/candidate pairs per binary order, with
fixed dataset/case order. Four Cargo profile locks exclude builders; no profiler
or GUI observes primary timing. Fixtures use warm synthetic in-memory data on
GPUI TestPlatform and make no real provider/account requests.

Handler timing covers entry through returned value. Full stdio covers parent
write/drain through the complete stdout frame, including the MCP process, Unix
socket and foreground native handler. Construction, request encoding, parent
JSON decoding and assertions are outside the clock.

| Family / boundary | Primary observations | Excluded warmups |
| --- | ---: | ---: |
| Generic handler | 5,760 | 960 |
| Generic full stdio | 8,880 | 1,480 |
| Projected handler | 30,240 | 5,040 |
| Projected full stdio | 30,240 | 5,040 |
| Dedicated Finance full stdio | 12,800 | 640 |

Generic/projected cases use 30 observations after five warmups. Finance full
stdio uses 200 after ten. The 1,600 dedicated Finance handler observations retain
three-decimal summaries after 160 warmups; their raw intervals were not captured.
The 800 fresh-process MCP transport initialization observations follow 16 excluded launches and
measure startup/handshake of the unchanged MCP executable with warm OS caches,
not native app startup.

### Health results and other read controls

These tables show **baseline → current p50 milliseconds**. Health timings use
six decimal places so the small empty-query differences remain visible. All
percentages use the unrounded values; rounded Finance handler data remain
three-decimal summaries. Generic datasets have
10,001 complete records per domain; Unicode-heavy Knowledge retains two complete
saved versions. Projected targets have 100,001 records, except Chat's 10,001.
Projected labels are Unicode-heavy; the dedicated transaction ledger is ASCII.

| Case / boundary | Baseline first / 1 | Candidate first / 1 | Baseline first / 2 | Candidate first / 2 |
| --- | ---: | ---: | ---: | ---: |
| health/sleep / first / handler | 0.047709 → 0.044250 | 0.047666 → 0.048166 | 0.048000 → 0.050000 | 0.044375 → 0.049584 |
| health/sleep / deep / handler | 0.048500 → 0.048959 | 0.048792 → 0.049167 | 0.049000 → 0.050958 | 0.048917 → 0.050417 |
| health/sleep / last / handler | 0.691375 → 0.559500 | 0.684834 → 0.560584 | 0.691416 → 0.589041 | 0.690792 → 0.568750 |
| health/sleep / none / handler | 0.439625 → 0.327625 | 0.433416 → 0.327625 | 0.442000 → 0.338958 | 0.436417 → 0.331125 |
| health/sleep / filtered-deep / handler | 1.228292 → 1.090084 | 1.218083 → 1.088750 | 1.291292 → 1.134208 | 1.224833 → 1.106459 |
| health/sleep / filtered-final / handler | 1.179250 → 1.043667 | 1.172375 → 1.043292 | 1.227250 → 1.066458 | 1.179958 → 1.054042 |
| health/sleep / hidden-miss / handler | 0.412625 → 0.352209 | 0.403916 → 0.351459 | 0.410250 → 0.357500 | 0.409542 → 0.350709 |
| health/sleep / first / stdio | 0.239208 → 0.240250 | 0.250792 → 0.249291 | 0.258000 → 0.242750 | 0.242625 → 0.243042 |
| health/sleep / deep / stdio | 0.237750 → 0.239792 | 0.238291 → 0.250000 | 0.253042 → 0.234250 | 0.237000 → 0.241292 |
| health/sleep / last / stdio | 0.854875 → 0.698792 | 0.845708 → 0.718417 | 0.866292 → 0.728500 | 0.841291 → 0.714417 |
| health/sleep / none / stdio | 0.588666 → 0.466583 | 0.579542 → 0.459250 | 0.623042 → 0.482375 | 0.582959 → 0.470666 |
| health/sleep / filtered-deep / stdio | 1.419500 → 1.349875 | 1.405042 → 1.297917 | 1.493625 → 1.356208 | 1.424750 → 1.294791 |
| health/sleep / filtered-final / stdio | 1.343625 → 1.173500 | 1.321708 → 1.194292 | 1.424167 → 1.216833 | 1.356333 → 1.178792 |
| health/sleep / hidden-miss / stdio | 0.556250 → 0.493167 | 0.553250 → 0.493667 | 0.608208 → 0.503000 | 0.554042 → 0.478833 |
| health/recovery / first / handler | 0.059083 → 0.058250 | 0.058167 → 0.058375 | 0.061000 → 0.057209 | 0.058042 → 0.058917 |
| health/recovery / deep / handler | 0.056667 → 0.055958 | 0.055375 → 0.056791 | 0.058458 → 0.055042 | 0.055458 → 0.056833 |
| health/recovery / last / handler | 0.696084 → 0.574834 | 0.690750 → 0.584959 | 0.720459 → 0.628083 | 0.709792 → 0.568750 |
| health/recovery / none / handler | 0.442792 → 0.328708 | 0.437209 → 0.325125 | 0.456959 → 0.335542 | 0.439209 → 0.326208 |
| health/recovery / filtered-deep / handler | 1.236791 → 1.099958 | 1.231584 → 1.097208 | 1.280917 → 1.158000 | 1.237792 → 1.098375 |
| health/recovery / filtered-final / handler | 1.184625 → 1.047750 | 1.179250 → 1.053250 | 1.292875 → 1.137458 | 1.184291 → 1.045959 |
| health/recovery / hidden-miss / handler | 0.414959 → 0.354042 | 0.410583 → 0.352750 | 0.437958 → 0.396333 | 0.411250 → 0.358417 |
| health/recovery / first / stdio | 0.269000 → 0.267292 | 0.264958 → 0.269417 | 0.295167 → 0.298750 | 0.293041 → 0.266875 |
| health/recovery / deep / stdio | 0.251084 → 0.250542 | 0.250916 → 0.252750 | 0.272750 → 0.257750 | 0.260709 → 0.253458 |
| health/recovery / last / stdio | 0.845458 → 0.703333 | 0.833792 → 0.726292 | 0.870209 → 0.749875 | 0.860333 → 0.693292 |
| health/recovery / none / stdio | 0.579417 → 0.470209 | 0.619583 → 0.468500 | 0.641458 → 0.503666 | 0.603917 → 0.466958 |
| health/recovery / filtered-deep / stdio | 1.430667 → 1.371084 | 1.525000 → 1.359250 | 1.518000 → 1.368584 | 1.447958 → 1.305625 |
| health/recovery / filtered-final / stdio | 1.278291 → 1.225833 | 1.424375 → 1.200208 | 1.407667 → 1.271750 | 1.383875 → 1.200333 |
| health/recovery / hidden-miss / stdio | 0.550334 → 0.493291 | 0.602583 → 0.487417 | 0.599708 → 0.529625 | 0.566917 → 0.487792 |

The earlier mixed Unicode miss remains a separate full-stdio control:

| Case / boundary | Baseline first / 1 | Candidate first / 1 | Baseline first / 2 | Candidate first / 2 |
| --- | ---: | ---: | ---: | ---: |
| Knowledge `ζ-unseen` / full stdio | 197.701 → 39.745 | 196.957 → 41.398 | 196.536 → 39.234 | 197.561 → 40.192 |

The dedicated Finance `missing` query retains its own handler and stdio controls:

| Case / boundary | Baseline first / 1 | Candidate first / 1 | Baseline first / 2 | Candidate first / 2 |
| --- | ---: | ---: | ---: | ---: |
| Finance missing / handler | 1.868 → 1.843 | 1.869 → 1.851 | 1.835 → 1.851 | 1.830 → 1.841 |
| Finance missing / stdio | 1.986 → 1.974 | 1.977 → 2.031 | 1.967 → 2.062 | 1.961 → 2.008 |

Unfiltered pages remain mixed. Sleep deep-page handler p50 is 0.8–4.0% higher in all four comparisons, although that empty-query path bypasses the changed predicates. Two projected full-stdio cases also have higher p50 in all four comparisons: Finance Accounts exact hidden-miss (0.6–4.5%) and Workouts deep-page (0.8–13.2%). Generic queries that still lowercase the entire serialized record and individual tails remain recorded. These are unresolved observations; directional repetition alone does not establish an implementation-caused regression.

Each cell is one baseline/current summary comparison for a specific case,
boundary, execution order and replicate. The 1,304 cells comprise 96 generic
handler, 148 generic full-stdio, 504 projected handler, 504 projected full-stdio,
16 dedicated Finance handler and 36 Finance full-stdio/initialization comparisons.
They correspond to the observation counts above and remain selectable in the
interactive results and raw comparisons. Nearest-rank p95/p99 remain separate;
p99 is the maximum for 30 observations. All observations retain their captured
form, with raw arrays except the dedicated Finance handler summaries. Requests
inside one process are not independent process replications. Small differences
are not statistical equivalence; no numerical noise tolerance is asserted.

Complete parsed reply fingerprints and frame sizes agree across all eight
process runs of each boundary family (four baseline, four candidate). Outer JSON
keys are canonicalized while inner record JSON text remains intact. Assertions
cover IDs, counts, offsets, saved-history matching, projection/redaction, excluded
content and grant revocation. All unfavorable candidate observations are retained.

### Same-binary calibration and interpretation

A later, separate diagnostic ran the exact original native executable under both
A and B labels. It used eight fresh processes, the same 18 projected targets and
seven cases, with two separate A/B pairs per label order. Its 30,240 handler and
30,240 full-stdio observations (5,040 excluded warmups each) are not pooled with
the candidate comparison. Complete replies and frame sizes agree with each other
and the primary experiment.

Despite identical code, six handler cases and ten full-stdio cases had higher B
p50 in all four comparisons. The table gives **same-binary A → B p50 percentage
change**, rather than an implementation speedup. Full data retain all 1,008 cells.

| Same-binary case / boundary | A first / 1 | B first / 1 | A first / 2 | B first / 2 |
| --- | ---: | ---: | ---: | ---: |
| health/sleep/deep / handler | +6.84% | -4.40% | +0.17% | +1.28% |
| health/workouts/deep / stdio | +3.58% | -6.46% | +1.05% | +1.47% |
| finance/accounts/exact/hidden-miss / stdio | +1.41% | -1.18% | -5.94% | +1.64% |
| finance/budgets/exact/none / stdio | +17.58% | +0.36% | +5.94% | +13.30% |
| finance/income_streams/redacted/none / handler | +21.89% | +4.74% | +2.21% | +6.77% |

Four higher medians alone are therefore insufficient to attribute a slowdown to
code. Earlier repeated-increase lists preserve observations and historical
selection decisions; they should not be treated as a causal regression gate.
This diagnostic does not explain the variability, establish a numerical acceptance
tolerance, characterize every candidate path, or prove equivalence. The current
untargeted controls remain unresolved; the broad action goal remains open.

The remaining Accounts exact excluded-content miss scans 100,001 Unicode-labelled
records. Its ASCII query contains `i`; names containing dotted-I enter a path
that creates a new owned normalized string before matching. Reusing temporary
normalization storage is therefore a concrete next hypothesis. Measure allocation
counts and transient capacity as well as latency; the source path alone does not
prove the time saved.

### Checks and remaining work

The frozen-source quick gate passes 2,446 Rust tests (37 ignored) and 38 site
tests, including all 12 query regressions. Formatting, native all-target/all-feature
and WASM Clippy, site types, benchmark self-checks and the normal optimized
native-performance compile check pass. The exact incremental patch verifies the
two caller edits and the unchanged query implementation.

No query/page/reply limit, permission rule, index or cache is introduced.
Normal native frames, physical input, browser paint, cold hydration, concurrency
and provider latency remain outside these measurements. Pure Unicode,
ambiguous/present-fragment queries and complete JSON serialization remain costs.
The full action-performance goal remains open; code is local and uncommitted.

Local evidence:

- [Generic comparisons](../target/performance-unicode-fragment9-comparison.json), [Finance comparisons](../target/performance-unicode-fragment9-finance-comparison.json) and [projected comparisons](../target/performance-unicode-fragment9-projected-comparison.json).
- [Native evidence verification](../target/performance-unicode-fragment9-native-verification.json), [gate receipt](../target/performance-unicode-fragment9-gate-receipt.json) and [incremental caller patch](../target/performance-unicode-fragment9-incremental-change.patch).
- [Record-scan round 2](../target/performance-record-scan2-probe-comparison.json), [round 3](../target/performance-record-scan3-probe-comparison.json) and [candidate selection](../target/performance-record-scan3-probe-selection.json).
- [Same-binary A/A results](../target/performance-projected-aa1-projected-comparison.json), [A/A verification](../target/performance-unicode-fragment9-aa-verification.json) and [interpretation note](../target/performance-unicode-fragment9-methodology-note.json).
- [Health code placement](../target/performance-unicode-fragment9-health-assembly.json) and [interactive results](../target/performance-unicode-fragment9-comparison.html).

The ledger adds 11 tool-to-experiment mappings for one shared current bundle:
five generic read tools and six projected read tools. These are partial variants,
not 11 independent experiments or whole-action acceptances. The 76 earlier ledger
entries are historical action/variant mappings, not a count of complete actions.
They retain their original measured pins; 62 have source/script hashes that differ
from the current tree. That provenance change does not itself imply a speed
change. The ledger now has 87 entries across 34 distinct action identities.

## Request-owned normalization buffer diagnostic

A standalone prototype removes the per-field normalization allocation for ASCII
queries that encounter `İ` or `K`. Production remains at the previously verified
Health caller checkpoint. Adoption requires validation through actual desktop read
handlers and complete MCP stdio calls, including unchanged search, paging and
permission behavior.

The macOS arm64 experiment is a standalone optimized Rust executable. It compares
the current matcher, including direct single-field Health calls, with a mutable
normalization buffer. Each page call creates one matcher and drops its buffer
before returning the page. Nothing is shared between page calls. The immutable
query keeps its existing 32-byte layout; empty scratch storage allocates nothing.

Across 94 actual-model scan/page cases, the separate allocation-counting build
reproduced identical counts in two opposite process orders. Twelve cases reduced
allocations; the other 82 had identical allocation metrics. For a 100,001-record
Unicode transaction scan using `hidden-perf-body`, current code made 100,005
allocations. The prototype made 5 allocations and 1 reallocation. Total requested
bytes fell from 3,189,589 to 750. Both versions freed every measured allocation.
These counts include query construction, matching, page projection, JSON encoding
and decoding, and result destruction; fixture construction and reference checks
are excluded.

Buffer reuse increases temporary retention. In that miss case, peak outstanding
requested bytes increased from 666 to 722. A separate synthetic 200,000-byte field
of repeated `İ` grew the buffer to 400,000 bytes, retained through later short
fields in the same matcher until it was dropped. This sequence tests retention
separately from the 94 page cases. These are requested allocation/capacity bytes,
not RSS, allocator overhead or transient memory inside `realloc`. No memory cap or
cross-request cache was introduced.

The uninstrumented release build ran 30 measured scans after 5 warmups per
case/method in each of two opposite process orders. The 12 allocation-reducing
cases had p50 values 23.0–34.5% lower across those orders. All 94 complete pages
matched the independent std lowercase reference, including offsets and redaction.
The raw intervals and p50/p95/p99 remain separate by order. Some control medians
were higher in both orders, including the ASCII last-merchant scan (+1.3% and
+2.6%). The earlier native same-binary A/A result remains relevant: directional
median changes alone establish neither causation nor equivalence. This diagnostic
supports testing the candidate, not accepting it as a native performance fix.

Validation passed 31 release tests and targeted Clippy with warnings denied. The
tests cover the existing query and paging regressions, reuse across changing
fields, and allocation-free scratch storage for empty/plain-ASCII/single-alias
paths. The initial all-targets Clippy command also included older experiment
binaries and failed on their unused methods; that failed receipt is retained.
The corrected invocation targets only this experiment. Timing and allocation
builds are separate; allocation instrumentation was absent from the timing build.
All four measurement processes finished. Production source pins still match the
last verified checkpoint, whose 2,446 Rust and 38 site tests remain historical
validation of that unchanged source. No new full application gate was run here.

The fixture uses actual Finance/Health model types and the unchanged production
paging module. It excludes GPUI, grants, bridge/MCP transport, generic full-JSON
record searches, normal UI frames, cold starts, providers and concurrent requests.
These component scans do not establish that any complete user action meets a
performance target. The existing action-evidence ledger is unchanged. Normal UI,
workflow, palette and HTTP coverage remain part of the open performance goal.
This pass added local prototype artifacts under `target/`. Earlier production
fixes remain uncommitted.

Local evidence:

- [Raw intervals, counts and complete reference pages](../target/performance-request-scratch1-comparison.json).
- [Prototype source snapshot and input pins](../target/performance-request-scratch1-manifest.json), [tests](../target/performance-request-scratch1-tests-receipt.json) and [targeted Clippy](../target/performance-request-scratch1-clippy-scoped-receipt.json).
- [Verification](../target/performance-request-scratch1-verification.json), [interactive diagnostic](../target/performance-request-scratch1-comparison.html) and [resume checkpoint](../target/performance-request-scratch1-work-state.json).

## Request-owned normalization in native read scans

The local candidate reuses normalization storage across fields and records. All
16 projected alias-stress variants have lower native-handler and full-MCP p50 in
four comparisons against current9. The candidate remains provisional because
several control medians are higher and two alias-stress stdio tail cells are
higher. Causation of the higher medians remains unverified. The implementation
is retained locally for control investigation; broader performance acceptance and
a merge remain pending.

The eight-file change gives each of the seven shared-query callers one mutable
matcher. This covers generic full-JSON records, Finance, Health, Meetings, Notes,
People and Chat. The immutable query retains its 32-byte layout. Only relevant
ASCII-source normalization grows the buffer; plain ASCII and single-alias fast
paths leave scratch unallocated. Each read releases its buffer; nothing is shared
between reads. The source diff leaves search fields, JSON/history serialization,
projections, offsets, page limits and permission branches unchanged. The tests
exercise Unicode equivalence, complete replies, paging, redaction and revocation
for the recorded cases; they do not prove every application action.

### Native evidence against current9

The baseline is the previously verified current9 executable, which already has
the direct single-field Health predicates. These measurements isolate the new
reuse implementation from that earlier combined comparison against originalC.
The baseline and candidate are different optimized native desktop test
executables. Both spawn byte-identical production MCP transport executables and
use the same runtime scripts on macOS arm64, with GPUI TestPlatform and synthetic
in-memory records. They do not measure
normal native frames or physical input.

All 32 native processes finished. The generic, Finance and projected families
retain two separate process comparisons in each opposite binary order, with
fixed case/data order. Raw samples, nearest-rank quantiles, complete response
fingerprints and frame sizes verify. Paging, grants, excluded content, exact and
redacted amounts, saved history and cleanup checks pass. No samples are pooled
across orders. Projected cases use 30 samples after 5 warmups; their p99 is the
maximum. Finance handler controls retain their existing 3-decimal summaries.

The 16 projected alias-stress variants search `hidden-perf-body` in public
metadata containing `İ` and `K`, with 100,001 records except 10,001 Chat threads.
Private content remains excluded. Across all four comparisons, their handler p50
is 14.7–42.2% lower and full-MCP p50 is 7.3–42.4% lower. Handler p95/p99 are lower
in all 64 such comparison cells. Full-MCP p95/p99 are higher in two cells: Budget
redacted in the first baseline-first comparison (p95 +4.7%, p99 +12.2%) and Chat
in the second baseline-first comparison (p95 +7.2%, p99 +0.7%).

Selected p50 changes are below. Columns retain the separate process comparisons;
negative values mean the candidate median was lower.

| Boundary and case | Baseline first 1 | Candidate first 1 | Baseline first 2 | Candidate first 2 |
| --- | ---: | ---: | ---: | ---: |
| Transaction exact handler, alias miss | -36.2% | -38.4% | -34.5% | -38.2% |
| Transaction exact MCP, alias miss | -37.2% | -40.9% | -38.7% | -34.9% |
| Chat handler, plain ASCII miss | +9.0% | +12.1% | +9.6% | +3.9% |
| Chat MCP, plain ASCII miss | +7.1% | +6.1% | +23.4% | +3.9% |
| Holding redacted handler, filtered final | +13.0% | +6.2% | +4.8% | +8.0% |

Controls remain unresolved. Fifteen projected handler cases and ten projected
stdio cases have higher p50 in all four comparisons. Generic/control results are
also mixed: the generic Knowledge `missing-kelvin` full-MCP case changes by +1.1%,
-3.8%, -3.8% and -3.0%. The earlier same-binary A/A experiment shows why repeated
median direction alone establishes neither causation nor equivalence. That
caution applies to improvements too. It supplies no numerical noise tolerance,
and it does not explain these observations. Retain the reuse
implementation locally to investigate these controls. Its performance trade-offs
remain unaccepted; the uncapped scan-time retention is one of those trade-offs.

### Allocation evidence and buffer lifetime

The earlier actual-model projected prototype reduced allocations in 12 of 94
cases; 82 cases had identical allocation metrics. A 100,001-record Unicode
transaction alias miss dropped from 100,005 allocations to 5 plus 1 reallocation.
That was a standalone diagnostic, separate from native timing.

This pass also measured the exact generic read function bodies and query modules
against an independent std lowercase reference, using KnowledgeDocument records
with two saved versions. The 36 cases include ASCII/Unicode data, empty/deep
pages, canonical JSON-field queries, history matches, invalid offsets and a
600,000-byte source body placed first or last. All complete pages and errors
match. Counts reproduce in two opposite process orders; every tracked allocation
is freed. Nine cases reduce allocations and 27 have identical allocation metrics.

For a 10,001-document Unicode alias miss, allocations fall from 420,048 to
410,048. Total requested bytes fall from 121,665,646 to 70,176,937. Peak outstanding
requested bytes increase from 19,081 to 24,151 during scanning. These are
allocator-requested bytes, not RSS or allocator overhead; realloc-internal memory
is excluded. Fixtures and expected reference results are outside the tracked
scope. The counts include final result destruction. No latency is taken from the
instrumented build.

The initial reuse candidate retained scratch through final response serialization.
The separate large-first JSON-field case exposed 1,203,442 extra peak requested
bytes. Its native build was intentionally terminated before any timing runs.
The revised generic reader drops the matcher immediately after scanning. The
same case now matches current9's 4,297,884-byte requested peak. Allocation savings
remain; ordinary Unicode misses still retain 5,070 extra peak bytes. No memory
cap or cross-request cache was added. Large fields can retain their grown scratch
capacity during scanning; the earlier standalone capacity diagnostic remains
applicable.

Correctness and build gates pass 2,450 Rust tests (37 ignored) and 38 site tests, including
16 query tests. Format, native/WASM Clippy, site types, benchmark self-checks and
the normal release check pass. The projected stage initially stopped because its
runner expected to create a manifest already frozen by setup. No projected
measurements had started. The corrected runner verified and reused those exact
pins, preserving the 24 completed generic/Finance runs and all later samples.

The evidence ledger adds 11 partial tool mappings to one shared experiment
bundle, retaining all 87 historical entries. It now has 98 entries across the
same 34 distinct action identities. These counts are neither independent
experiments nor complete user actions. Source and script pins total 1,152 paths;
30 compiled assets are pinned separately. Historical source-pin differences are
provenance changes, not proof of speed changes. Production changes remain local
and uncommitted; the full all-actions performance goal remains open.

Local evidence:

- [Native verification](../target/performance-request-scratch3-native-verification.json), [gate](../target/performance-request-scratch3-gate-receipt.json), [incremental source patch](../target/performance-request-scratch3-source-change.patch) and [provisional decision](../target/performance-request-scratch3-candidate-status.json).
- [Generic timings](../target/performance-request-scratch3-comparison.json), [Finance controls](../target/performance-request-scratch3-finance-comparison.json), [projected timings](../target/performance-request-scratch3-projected-comparison.json) and [interactive comparisons](../target/performance-request-scratch3-comparison.html).
- [Initial generic memory diagnostic](../target/performance-generic-scratch2-comparison.json), [revised diagnostic](../target/performance-generic-scratch3-comparison.json) and [allocation verification](../target/performance-generic-scratch3-verification.json).
- [Final verification](../target/performance-request-scratch3-final-verification.json), [resume state](../target/performance-request-scratch3-work-state.json) and [remaining frontier](../target/performance-request-scratch3-next-action-frontier.json).

## Native workflow tools and pending-review scans

The three previously unmapped workflow tools now have native handler and full-MCP
measurements. A status-first pending-proposal scan removes repeated passes over
inactive history. At 10,001 workflows, submission and undo-review requests have
substantially lower observed latency in both binary orders. The fix is retained
locally on the strength of removed repeated work, exact owner-selection tests and
paired large-history results. Higher controls, read latency and remaining action
variants are still open requirements for broader acceptance and merge. No numerical
acceptance tolerance has been established.

### Coverage and preserved behavior

The fixture exercises `bruhs_propose_workflow`, `bruhs_workflow_status` and
`bruhs_request_workflow_rollback`. Sixteen cases cover allowed/blocked previews,
fresh submission, idempotent replay, payload conflict, status, revoked grants,
unknown session ownership, unsaved progress, undo eligibility, requesting review,
repeat pending review, declined review and changed-record rejection.

Each case runs with 1 and 10,001 workflows. One Capture record and completed
step/job are seeded; fresh proposals contain ordered Capture/Task operations.
Session ownership contains up to 99 proposals, reserving one of the existing
100 slots for a new submission. Timed calls never approve or execute a proposal
or delete a record. Failure-state flags and changed records are prepared by the
fixture outside the timer. Saves use a private temporary workspace file, with
seed-save acknowledgements and fixture resets outside the measured interval.
New submission returns provisional previewed status with `durable=false`; save
completion is a separate, unmeasured latency.

The native before/after comparison uses the same fixture and production MCP
transport binary. Only the pending-proposal selector production code changes;
two selector regression tests are also added. Both workflow executables already
contain the restored direct-matcher inline boundary described below.

### Work removed and native results

The previous selector iterated session proposals and searched workflow history
for each one. With 99 owned proposals and 10,001 inactive workflows, its source
work model requires 990,099 ID comparisons per pending check. The new multi-owner
path visits history once and compares IDs only for previewed workflows. It keeps
the original proposal-map priority when several previews match. Zero/single-owner
behavior is unchanged. No cache, index, allocation, limit, grant, approval or
persistence-policy change is introduced.

Two new tests compare the exact selected owner, including pointer identity,
against the original nested lookup through reordered/duplicate history, owner
insertions/removals, zero/single owners and unowned previews. The separate release
selector diagnostic uses actual workflow models and the production selector as
reference. Its no-preview 99-owner/10,001-workflow case changes from about 1.9 ms
to 6.5 microseconds in both process orders. Those batch measurements are separate
from the actual native results below.

All four native processes finished in ABBA binary order, one comparison per
opposite execution order. Case/history order is fixed. Each boundary retains
3,840 measured intervals plus 640 excluded warmups: 30 samples after 5 warmups for
each case/cohort. Nearest-rank p99 is the maximum of 30 samples. Request encoding,
fixture preparation/save waits, parent validation and startup are excluded.
Every complete reply fingerprint and frame size agrees across cohorts. Only a
validated generated workflow UUID is normalized in submission replies; all other
content is retained.

| 10,001-workflow case and boundary | Baseline first, before / after p50 | Candidate first, before / after p50 |
| --- | ---: | ---: |
| Submission, handler | 2.792500 / 0.173541 ms | 2.224667 / 0.153625 ms |
| Submission, full MCP | 3.818791 / 0.642042 ms | 3.781583 / 0.584417 ms |
| Undo-review request, handler | 1.955625 / 0.046750 ms | 2.000084 / 0.032625 ms |
| Undo-review request, full MCP | 2.108875 / 0.205250 ms | 2.078083 / 0.233750 ms |

Submission p50 is 93.1–93.8% lower in handlers and 83.2–84.5% lower through MCP.
Undo-review request p50 is 97.6–98.4% lower in handlers and 88.8–90.3% lower
through MCP. Targeted p95/p99 are also lower in both orders. This is descriptive evidence;
30 samples and a maximum-based p99 do not establish tail stability.

Other controls remain visible. Out of 32 case/history variants per boundary, ten
handler variants and nine full-MCP variants have higher p50 in both comparisons. Large-history preview handler p50 changes from
0.000750 to 0.001792 ms and from 0.000959 to 0.002250 ms. The small-history
changed-record rejection MCP case changes from 0.180167 to 0.232750 ms and from
0.141541 to 0.232333 ms. Repeated direction alone does not establish causation or
equivalence. The earlier same-binary read A/A result supplies no numerical
tolerance for this workflow fixture. These observations remain unresolved.

The scan still has O(N + P*M) comparison work when P workflows are previewed and
M proposals are owned. Many unrelated previews remain a separate shape to
investigate. An indexed prototype speeds that standalone shape but adds a
temporary map; it was not adopted. The retained path uses no new allocation and
keeps the single-owner path. No broad optimality claim is made.

### Read controls and validation

After the earlier scratch3 read measurements, disassembly found that its facade
had added three direct matcher calls per Chat record and two in Health. The
existing forced inline boundary at `Matcher::matches` was restored before either
workflow executable was built. Both workflow measurements therefore include that
restoration. Static inspection of the workflow baseline verifies removal of those
call sites. This verifies the code-path change; latency in the
read controls still needs a new paired measurement. The earlier scratch3 control
observations remain historical evidence, not resolved regressions.

Correctness/build gates pass 2,453 Rust tests, with 38 additional ignored tests,
and 38 site tests,
including all 16 workflow fixture cases and the new selector tests. Format,
native/WASM Clippy, site types, benchmark self-checks and the normal release
check pass. A separate debug full-MCP smoke covered all 16 cases before release
measurement. Failed fixture-development checks remain archived and outside the
release dataset. Every cohort is pinned to its own compiled source.

The ledger adds three partial mappings to one shared workflow experiment. It
retains all 98 prior entries and now has 101 entries across 37 action identities.
All 31 discovered MCP names have at least one partial mapping; this closes a
mapping gap, not complete performance coverage. Native bridge and standalone
modes, long steps/jobs/records, other rollback domains, many previews, cold I/O,
concurrency, execution/undo and save-completion latency remain distinct work.
HTTP, palette/slash and normal native/browser actions also remain open.
Source/runtime pins cover 1,152 paths, with 30 assets separately pinned. Source
changes remain local and uncommitted; nothing was merged.

Local evidence:

- [Native comparisons and raw intervals](../target/performance-workflow-native2-comparison.json), [retention decision](../target/performance-workflow-native2-selection.json) and [production patch](../target/performance-workflow-native2-production-change.patch).
- [Candidate gate](../target/performance-workflow-native2-after-gate-receipt.json), [selector diagnostic](../target/performance-pending-review-probe1-comparison.json) and [inline call verification](../target/performance-workflow-native2-inline-call-verification.json).
- [Interactive workflow results](../target/performance-workflow-native2-comparison.html), [final verification](../target/performance-workflow-native2-final-verification.json) and [resume state](../target/performance-workflow-native2-work-state.json).

## Read controls after restoring direct matcher inlining

The current build has lower Chat ASCII-miss handler p50 in all four comparisons
(0.8-10.5% lower), while its full-MCP Chat and Health results remain mixed.
The restored direct matcher inline boundary stays local and provisional;
higher controls and broader action acceptance remain open. The HTTP inventory
also now includes the shared live-events route it previously missed.

### Comparison and results

The before executable is the earlier scratch3 candidate. The after executable
is the current workflow-native2 candidate. Besides the inline attribute and its
comment, the after build includes the pending-proposal selector change and a new
workflow test module. All 13 read fixture source files are identical. The read
fixtures have no workflows/proposals; binary layout still differs, so this
comparison cannot attribute every timing change solely to the inline attribute.
It also does not replace the earlier comparison against the pre-scratch build.

All 32 native processes passed: eight generic, eight Finance handler, eight
Finance full-MCP and eight projected-read processes. Each run group provides two
separate comparisons in each opposite binary order, with fixed case/data order. Generic
reads use 10,001 records per domain. Projected reads use 18 targets and seven
query/page cases, with 100,001 records except 10,001 Chat threads. They retain
30 measured intervals after five warmups per case/cohort. Finance controls retain
200 full-MCP intervals after ten warmups, plus existing rounded handler summaries.
The 24 generic handler diagnostics use ASCII queries. The 37 full-MCP cases
comprise 20 ASCII-dataset and 17 Unicode-Knowledge cases with additional queries.

The table lists p50 changes in baseline-first pair 1, candidate-first pair 1,
baseline-first pair 2 and candidate-first pair 2 order. Negative values mean lower
observed latency in the current build. Absolute timings and every control remain
available in the linked comparison files and chart.

| Read control | Native handler p50 changes | Full MCP p50 changes |
| --- | ---: | ---: |
| Chat ASCII miss | -1.2%, -10.5%, -0.8%, -4.7% | +16.3%, -5.5%, +15.5%, -18.5% |
| Chat final match | -0.5%, -0.2%, +1.2%, +1.7% | +12.3%, -8.2%, +21.3%, -13.1% |
| Sleep ASCII miss | +1.1%, +0.7%, -3.1%, +0.8% | -1.4%, -2.8%, +11.4%, +4.9% |
| Recovery ASCII miss | +0.7%, -1.0%, +1.3%, +0.9% | +1.5%, -2.6%, +5.7%, +2.6% |

Chat ASCII-miss handler p95/p99 are higher in three of the four comparisons.
In the first pair, p95 changes from 0.259084 to 0.353042 ms and p99 from
0.259959 to 0.407167 ms. Lower medians do not resolve those observations.

The full control matrix remains mixed:

| Boundary | Variants | Lower p50 in all four comparisons | Higher p50 in all four comparisons |
| --- | ---: | ---: | ---: |
| Generic handler | 24 | 5 | 3 |
| Generic full MCP | 37 | 8 | 0 |
| Projected handler | 126 | 9 | 10 |
| Projected full MCP | 126 | 8 | 12 |
| Finance rounded handler | 4 | 1 | 0 |
| Finance MCP and fixed initialization | 9 | 0 | 1 |

For example, the projected-read exact Finance account filtered-deep handler is 4.3%, 7.2%,
4.6% and 13.8% higher across the four comparisons. Its first pair changes from
2.326625 to 2.426417 ms. The exact income-stream ASCII-miss full-MCP case is
5.4%, 2.2%, 3.4% and 5.0% higher, with a first pair of 1.016084 and 1.071458 ms.
These are unresolved observations.

Median directions establish neither causation nor equivalence. No numerical
noise tolerance or performance acceptance threshold has been established. The
earlier same-binary A/A read study does not supply one for this comparison.
Nearest-rank p99 is the maximum of 30 samples in the generic/projected fixtures;
individual tails remain inspectable rather than proof of tail stability.

### Evidence and limits

The bundle retains 88,720 raw intervals, including 800 fixed-MCP initialization
observations, plus 1,600 Finance handler observations as three-decimal summaries.
There are 13,336 excluded warmups in total. Fixed-MCP initialization uses warm
OS caches and is separate from native app startup. All complete response
fingerprints/frame sizes agree across eight cohorts for each of the three full-MCP
families (generic, Finance and projected). Paging, history,
IDs, redaction, excluded fields, grant revocation and cleanup checks pass.

Original compiled manifests and build receipts are preserved byte for byte:
1,150 before and 1,151 after source pins, plus 30 assets each. Supplemental read
runtime manifests identify the unchanged Python helpers actually used by this
follow-up without rewriting the producers' build metadata. The combined current
source/runtime set contains 1,154 paths: 1,151 build-source paths plus three
additional runtime scripts. The 30 assets are separate. The exact after build already passed
2,453 Rust tests, with 38 additional ignored tests, and 38 site tests. Its gate
and normal release-check receipts are reused after source revalidation; no new
compilation or Rust gate is claimed here. The inventory change is checked by
running its discovery and complete ledger-mapping paths separately.

The direct facade dispatch calls identified earlier remain removed in the
archived current build. Local retention preserves the previous direct matcher
inline contract through the scratch facade. Dispatch removal and passing output
checks are recorded separately from performance acceptance; the implementation
remains provisional while the higher controls and tails are investigated. This phase adds no
allocation/RSS measurement; the earlier scratch-buffer retention tradeoff remains.
Normal frames, physical input, cold hydration, providers and concurrency remain
outside these warm GPUI TestPlatform and MCP measurements.

### HTTP coverage correction and next fixture

The source catalogue now discovers 95 explicit HTTP method/path entries. All 94
previous literal-path entries remain; `GET /api/events/stream` is added by resolving
`bruhs_models::api::live_events::PATH` at its source definition. Unknown path
expressions or nested routers now fail discovery instead of being silently skipped.
Implicit HEAD/CORS behavior, fallback and semantic variants still require review.

All 95 entries have source-pinned handler definitions and a fixture plan. The first
planned fixture calls the actual router with private task state, in-memory
credentials/connections and an unconfigured calendar using temporary storage.
It covers health, task reads/deletion and task/combined SSE frames. Finite body
completion, stream handshakes/change delivery and WebSocket lifecycle use separate
boundaries. No HTTP latency has been measured in this phase.

The ledger preserves its 101 prior entries and adds 11 partial read mappings to
this single shared experiment. Its 112 entries still represent 37 distinct
action identities. Repeated mappings add no samples and establish no completion
percentage. Source changes remain local and uncommitted; nothing was merged.

Local evidence:

- [Native verification](../target/performance-request-scratch4-native-verification.json), [comparison scope](../target/performance-request-scratch4-comparison-scope.json) and [summary](../target/performance-request-scratch4-summary.json).
- [Generic results](../target/performance-request-scratch4-comparison.json), [projected results](../target/performance-request-scratch4-projected-comparison.json), [Finance controls](../target/performance-request-scratch4-finance-comparison.json) and [interactive comparisons](../target/performance-request-scratch4-comparison.html).
- [HTTP discovery check](../target/performance-http-actions1-discovery.json), [handler/fixture plan](../target/performance-http-actions1-plan.json) and [resume state](../target/performance-request-scratch4-work-state.json).

## Actual HTTP task routes and event-stream baselines

Five HTTP actions now have measured loopback baselines through the actual router.
At 10,001 running tasks, full task-list p50 ranges from 7.888 to 8.704 ms across
four cohorts; full task snapshots through SSE range roughly 12.1-12.7 ms.
These observations identify work to isolate. This phase changes test coverage
and evidence; it makes no production optimization or speedup claim.

### Actions, fixture and timing boundaries

The fixture covers `GET /api/health`, `GET /api/tasks`,
`DELETE /api/tasks/{id}`, `GET /api/tasks/stream` and `GET /api/events/stream`.
Health has one empty-state case. Each other route runs its cases with 0, 1 and
10,001 running tasks, giving 31 case/count variants: all/thread-hit/thread-miss
reads; finished/running/missing deletion; and stream opening/change delivery.
The sparse thread has one matching task when the registry is nonempty. Task
metadata includes Unicode; log tails are empty. No shell process is launched.

The actual router retains access policy, CORS, body limits and response headers.
Its state uses in-memory credentials/connections/tasks, an empty meeting store
and an unconfigured calendar over private temporary storage. The fixture calls
neither `main()` nor background startup. Capture/provider handlers are never
requested. The private loopback listener and its connections shut down after
each fixture; no persistent server is left running.

Finite-response intervals start at client execution and end after the complete
body. Stream-opening intervals include the connection and complete expected
frames: one task snapshot, or task/calendar/meeting frames on the combined route.
Changed-stream intervals start before preparing/registering a synthetic task after
opening frames have been drained, and end at the complete changed task frame.
They include label/metadata preparation immediately before registry registration;
they do not measure agent task creation. Table counts describe the initial registry.
Changed snapshots contain one additional task, so the largest contains 10,002.
Deletion also prepares one additional target outside
the timer. Request construction, semantic validation, normalization, hashing,
seed/reset and server startup/shutdown are excluded. Body/frame accumulation and
delimiter detection are included. Serial requests use two Tokio workers so the
fixture client/server can run concurrently; this is not production runtime sizing.
The same pooled client is used within a fixture. Finite bodies are drained for
connection reuse. Each stream sample opens and drops its response; setup cost is
included in opening intervals and excluded from changed-stream intervals.

### Measured baseline

Four fresh processes use the same preserved optimized test executable: two in
forward case/count order and two in reverse order. Each variant retains 30 raw
intervals after five excluded warmups. There are 3,720 primary intervals and 620
excluded warmups. A separate release smoke run uses two observations after one
warmup per variant and contributes no primary samples.

The table shows the range of the four cohort p50 values in milliseconds, rounded
for display. It does not pool samples or establish a noise/acceptance tolerance.

| Case | 0 running tasks | 1 running task | 10,001 running tasks |
| --- | ---: | ---: | ---: |
| Task list, all | 0.046-0.085 | 0.041-0.049 | 7.888-8.704 |
| Task list, sparse thread | 0.046-0.079 | 0.037-0.053 | 0.160-0.203 |
| Task list, missing thread | 0.045-0.087 | 0.046-0.051 | 0.153-0.179 |
| Delete finished | 0.043-0.083 | 0.044-0.056 | 0.123-0.138 |
| Delete running conflict | 0.047-0.075 | 0.039-0.052 | 0.113-0.147 |
| Delete missing | 0.044-0.075 | 0.042-0.049 | 0.136-0.146 |
| Tasks SSE opening | 0.152-0.204 | 0.155-0.171 | 12.214-12.656 |
| Combined SSE opening | 0.172-0.205 | 0.179-0.193 | 12.161-12.686 |
| Tasks SSE change | 0.023-0.029 | 0.028-0.052 | 12.113-12.643 |
| Combined SSE change | 0.024-0.030 | 0.029-0.062 | 12.127-12.418 |

Health p50 ranges from 0.069-0.097 ms. The 10,001-task finite list is
2,959,187 decoded body bytes; the task SSE opening is 2,959,195 frame bytes.
Nearest-rank p99 is the maximum of 30 observations; no tail-stability or
whole-action acceptance claim follows from this baseline.

### Payload checks, validation and next investigation

Every returned task object is first compared with all seeded model fields and
exact membership. Only verified generated IDs/start/end timestamps are then
replaced, and unordered task arrays are sorted. Error UUID replacement follows
exact message validation. Complete normalized payload SHA-256 values, decoded
body/frame byte counts and statuses agree across all four cohorts. Stream names
and handshakes, finite JSON types, security/cache headers and fixture cleanup are
checked separately. These are normalized payload fingerprints, not raw-wire hashes.
Combined opening frames include the task payload and named calendar/meeting
handshakes. Both handshake JSON objects are checked in full against
`{"kind":"stream_opened"}` before the combined payload is fingerprinted.

Three ordinary actual-router tests cover the case matrix, origin rejection and
the combined-stream resume flag. The quick workspace gate passes 2,456 Rust tests
with 39 additional ignored tests and 38 site tests. Format, native/WASM Clippy,
site types and benchmark self-checks pass; the normal server release check also
passes. A wrapper summary parser initially expected another Node reporter format.
The gate itself had succeeded; its retained logs supplied the corrected receipt
before compilation continued. No primary samples were collected during that failure.

The executable, Cargo receipt and 1,153-path workspace source snapshot are
archived. The snapshot was captured before this gate/build and rechecked before
and after every cohort. It matches the measured build's frozen sources, includes
unused workspace files and is not a transitive dependency graph. Compared with
the preceding archived read-control build, the only existing-file changes are the test-module
declaration, the existing workspace SHA-256 dependency added for tests and its
lockfile entry. Two new fixture source files contain the tests and timing harness.

Local Axum 0.8.9 source shows that `Event::json_data` sends serializer writes
through `EventDataWriter`. Bulk JSON encoding followed by `Event::data` is a
candidate CPU/allocation tradeoff. The finite/SSE timing gap does not isolate
that writer because client frame accumulation differs too. Compare construction
cost, exact raw SSE-frame parity and allocation/peak memory before changing
production, then repeat the actual HTTP measurement. Remaining HTTP routes still
need coverage, including persistent people/meeting reads and filesystem/git actions.

The ledger retains its 112 prior entries and adds five partial HTTP mappings to
this one shared experiment: 117 entries across 42 distinct action identities.
The catalogue still discovers 95 explicit HTTP method/path entries. Five route
mappings do not imply complete variants or an audit completion percentage.
Browser input/paint, normal server startup, cold storage, large log tails,
concurrency and provider/capture latency remain open. Changes are local and
uncommitted; nothing was merged.

Local evidence:

- [Baseline samples and payload fingerprints](../target/performance-http-native1-baseline.json), [source manifest](../target/performance-http-native1-manifest.json) and [interactive measurements](../target/performance-http-native1-comparison.html).
- [Gate receipt](../target/performance-http-native1-gate-receipt.json), [release build receipt](../target/performance-http-native1-build-receipt.json) and [SSE source investigation](../target/performance-http-native1-sse-source-review.json).
- [Resume state](../target/performance-http-native1-work-state.json).

## Buffered task event encoding

Task streams now batch JSON serializer writes before SSE line framing. With
10,001 initial running tasks, all four paired HTTP observations have lower
opening and changed-frame p50, p95 and p99. Opening p50 falls 28-31%; changed-frame
p50 falls 34-37%. The change remains local and uncommitted. Finite-route controls
and memory tradeoffs remain open; this is partial task-stream evidence.

### Change and constructor evidence

Axum 0.8.9 sends each JSON serializer write through its SSE delimiter scanner.
The local patch exposes that existing byte writer as a safe borrowed
`std::io::Write`. Task snapshots wrap it in an 8 KiB `BufWriter`, explicitly
flush, then return the event. Exact empty task lists keep unbuffered encoding.
Default `json_data` behavior elsewhere remains unbuffered. Watch registration,
snapshot cloning, full payloads, event names, errors and keepalives retain their
existing behavior. Serialization errors drop the partial event without yielding
a frame. No unsafe production code is added.

The [reproducible constructor study](../bench/sse-encoding-probe/README.md)
compares nine methods across 34 case/count variants in two method/data orders.
Two timing processes retain 18,360 intervals after 3,060 excluded warmups;
two separate allocation processes supply 612 observations. Actual task models
include ordinary/escaped text, long CRLF logs and 64 KiB ASCII/Unicode fields,
with named and unnamed events. Every frame matches independently framed model
JSON byte for byte. Additional tests cover pretty raw JSON, small buffer
boundaries and partial serialization failures.

The following table uses unnamed events. Times are forward/reverse p50 in
milliseconds. Peak is live requested allocation bytes during construction and
destruction, excluding prebuilt input models. These are constructor measurements,
not HTTP intervals or RSS.

| Fixture | Original p50 | Byte buffer p50 | Original peak | Byte buffer peak |
| --- | ---: | ---: | ---: | ---: |
| Ordinary, 1 task | 0.002209 / 0.000875 | 0.000958 / 0.000416 | 400 | 8,491 |
| Ordinary, 200 tasks | 0.136084 / 0.134625 | 0.047583 / 0.047667 | 102,400 | 73,704 |
| Ordinary, 10,001 tasks | 6.686625 / 6.689541 | 2.520583 / 2.489583 | 3,276,800 | 4,200,960 |
| Escaped, 10,001 tasks | 8.490334 / 8.618625 | 2.999958 / 2.880709 | 6,553,600 | 4,200,448 |
| Single 64 KiB ASCII field | 0.023083 / 0.023208 | 0.022625 / 0.023750 | 131,664 | 139,856 |
| Single 64 KiB Unicode field | 0.024292 / 0.023875 | 0.022625 / 0.023542 | 131,664 | 139,856 |

A full temporary JSON string was rejected: ordinary 10,001-task peak rises to
10,112,690 bytes. Checked string adapters introduce repeated UTF-8 work; the
8 KiB standard adapter takes 0.056875 / 0.059000 ms on the long Unicode case.
The selected byte adapter avoids that conversion. The long ASCII control remains
mixed. Its 8 KiB configuration follows the checked-buffer sweep from 256 through
16,384 bytes and subsequent byte-adapter validation at 8 KiB; it is not a universal
optimum. Peak output growth depends on chunking, so the extra memory can exceed
the staging allocation. One small task adds 8,091 peak requested bytes; the
ordinary large case adds 924,160. The staging buffer does not cap event size.
The byte adapter's own size sweep and concurrent-stream memory budget remain
open; 8 KiB is the measured candidate configuration.
Counters exclude allocator rounding/internal realloc copies; total requested
bytes count each realloc's full new requested size.

### Actual HTTP comparison

The preserved baseline and candidate use the identical actual-router fixture
and Rust toolchain on arm64. Four pairs each measure 31 variants with 30 retained
intervals after five warmups: 7,440 primary intervals and 1,240 excluded warmups.
A separate candidate smoke run contributes 62 intervals after 31 warmups and is
excluded from primary results. Forward pairs run baseline then candidate; reverse
pairs run candidate then baseline, also reversing case/count order. Executable
order and case order vary together; these observations do not isolate drift or
establish independent statistical replication.

The fixture covers health, task-list reads, task deletion, task SSE and combined
SSE through the five routes described in the [HTTP baseline](#actual-http-task-routes-and-event-stream-baselines).
Finite responses include the complete body. Opened streams include the complete
initial frames. Changed streams include synthetic task preparation/registration
through the complete changed frame after opening frames were drained. Initial
counts are 0, 1 and 10,001; changed snapshots contain one additional task. Requests
are serial on two Tokio workers. Validation, normalization, hashing, seed/reset
and fixture startup/shutdown stay outside the intervals; frame accumulation and
delimiter detection stay inside.

The table reports ranges across four cohort p50 values in milliseconds. The
reduction is the range of paired p50 percentage reductions, not a ratio of range
endpoints or pooled samples. Counts below are the initial 10,001 running tasks.

| Case | Baseline p50 | Buffered p50 | Paired reduction |
| --- | ---: | ---: | ---: |
| Tasks SSE opening | 12.137-12.325 | 8.367-8.643 | 29.0-31.4% |
| Combined SSE opening | 12.115-12.293 | 8.574-8.860 | 27.9-30.2% |
| Tasks SSE change | 11.961-12.138 | 7.660-7.942 | 34.0-36.7% |
| Combined SSE change | 12.168-12.336 | 7.817-7.934 | 35.1-36.4% |

All 16 large-stream pair/case cells have lower p95 and p99 too. With 30 samples,
p99 is the maximum observation; it is not a stable tail guarantee. Small/empty
streams and finite controls remain mixed. At 10,001 tasks, the unchanged finite
list has higher p50 in two pairs and higher p95/p99 in three. Running-task deletion
has higher p50 in all four pairs (1.6-8.6%); finished deletion has higher p99 in all
four. These observations establish neither equivalence nor an accepted regression
tolerance. Complete controls and tails remain in the interactive results.
The repeated control increases require follow-up before merge acceptance; no
numerical tolerance has been established to dismiss them.

All 31 normalized complete payload fingerprints, decoded body/frame byte counts
and statuses agree across the eight primary processes and smoke run. Every task
field and membership is checked before normalizing verified generated IDs/times
and unordered task-array order. Event names, full calendar/meeting handshakes,
security/cache headers and fixture cleanup also pass. These HTTP fingerprints
are normalized payload hashes; constructor tests separately establish raw-frame
parity. No provider, capture process or personal store is used.

### Validation and checkpoint

The quick workspace gate passes 2,459 Rust tests, with 39 additional ignored,
and 38 site tests. Format, native/WASM Clippy, site types and benchmark checks
pass. The normal server release check and standalone constructor tests/Clippy
pass. Full merge-gate checks have not been rerun for this change.

The candidate's 1,250-path source snapshot was frozen before gate/build and
rechecked around every process. It is a workspace superset, not a transitive
dependency graph. The baseline's archived 1,153 paths and executable remain
unchanged. Existing-file differences are the root manifest/lockfile and task
stream caller; additions include the encoder, vendored dependency and standalone
study. The [Axum provenance note](../third_party/axum/VENDORED.md) records the
verified registry archive checksum, retained license and narrow two-file patch.

The ledger preserves 117 prior entries and adds five partial follow-up mappings
to this shared HTTP bundle, including finite controls: 122 entries across the
same 42 action identities. Repeated mappings add no samples or coverage breadth.
The inventory still discovers 95 explicit HTTP entries, of which five have HTTP
mappings. Remaining HTTP identities, persistent people/meeting and filesystem/git
paths, normal browser/native actions, cold storage, concurrent streams and large
log-tail HTTP distributions remain open. The targeted fix is retained locally
because it removes verified framing work and improves the measured large-stream
path with exact protocol checks. It does not establish fastest-possible behavior
for every action. Retaining it locally keeps a reviewable candidate; merge
acceptance remains unresolved. No commit, merge or push was made; measurement
workers are stopped.

Local evidence:

- [Paired HTTP observations](../target/performance-http-buffered1-comparison.json), [interactive results](../target/performance-http-buffered1-comparison.html) and [source manifest](../target/performance-http-buffered1-manifest.json).
- [Constructor observations and allocations](../target/performance-sse-bench1/comparison.json), [gate receipt](../target/performance-http-buffered1-gate-receipt.json) and [release build receipt](../target/performance-http-buffered1-build-receipt.json).
- [Resume state](../target/performance-http-buffered1-work-state.json).

## Persistent and filesystem HTTP reads

Thirteen more HTTP actions now have partial measured evidence through the actual
router. A directory-browsing candidate reduces the 10,001-directory p50 by
28.5% and 29.2% in opposite execution orders. The control with no visible
subdirectories is higher in both orders, so the change remains a local candidate with acceptance
unresolved. The full performance goal remains open.

### Actions and isolated fixtures

People reads cover listing, first/missing identity lookup, first/after-cursor
version pages, reverse-ordered 64-ID profile batches, missing profiles, duplicate
batch rejection and unmerged identity resolution. Meeting reads cover listing
and first/missing identity lookup. These map to seven method/path identities:
`GET /api/people`, `GET /api/people/{id}`, `GET /api/people/versions`,
`POST /api/people/profiles`, `POST /api/people/resolve-merges`,
`GET /api/meetings` and `GET /api/meetings/{id}`.

Each store fixture contains 0, 1 or 1,001 contacts and the same number of meetings,
separately using the in-memory store and private temporary on-disk SQLite.
Contact data includes aliases, 64 voice values, an observation, tracked state and
fixed times. Meetings include speakers, ended state and a summary. One additional
fixture per backend contains one contact and one meeting, both with a 64 KiB
Unicode field in the observation and summary respectively. These two additional
fixtures give eight store fixtures. Their twelve cases
give 96 store variants. First-identity lookups are missing at count zero; profile
batches contain only existing matches while retaining request order. Merge reads
have no redirects, so merged-chain behavior remains unmeasured.

Six filesystem identities cover `POST /api/agent/list-dir`,
`POST /api/agent/files/children`, `POST /api/agent/validate-path`,
`POST /api/agent/git-branches`, `POST /api/agent/git-status` and
`POST /api/agent/git-diff`. Nine cases run with 0, 1 or 10,001 visible child
directories. Each directory fixture also contains one hidden directory, a regular
file, a directory symlink and a broken symlink. The nonempty sets include a Unicode
name with a space. The real home boundary is retained; all filesystem fixtures
live under a new private directory in this workspace's `target`, and `HOME` is
never replaced. Validation covers a directory, regular file and missing path;
children also covers a missing project.

Git always uses a separate one-file unborn repository with a staged base and
changed working copy. Its branch response is empty, status checks both index and
working-tree state, and diff exercises the existing unborn-HEAD fallback. Git
setup uses an empty local template and local deterministic settings. No commit,
remote or real project worktree is modified. Repeating these Git cases at each
directory count supplies controls; it does not measure repository-size scaling.
The filesystem matrix adds 27 variants, giving 123 in total.

### Timing, output checks and directory change

Two before/after pairs run four fresh optimized test processes. Forward executes
baseline then candidate; reverse executes candidate then baseline and reverses
case/backend/count order. Executable and case order vary together, so drift is
not independently isolated. Each variant retains 30 observations after five
warmups: 14,760 primary intervals and 2,460 excluded warmups. Separate baseline
and candidate smoke runs each use two observations after one warmup per variant,
contributing 492 excluded intervals and 246 warmups together.

Serial requests use a pooled private loopback client and two Tokio workers.
Intervals cover client execution through the complete response body. Request
construction, seed/setup, JSON/header/full-payload checks, canonicalization,
hashing and cleanup are excluded. The production router retains its access,
CORS, body-limit and response-header layers. The fixture never calls application
startup, providers or capture, and uses no personal store. SQLite and filesystem
results are warm-cache observations, not OS-cold storage or UI latency.

Every complete JSON response must equal the independently constructed expected
payload, including array order, before fingerprinting. Fixed synthetic model
IDs/times need no normalization. Only known fixture paths are replaced after full
equality; raw decoded body lengths and statuses remain separate. All 123 full
canonical fingerprints, lengths and statuses agree across the four primary and
two smoke processes. Headers and cleanup also pass. Ordinary tests additionally
check outside-home rejection and the existing regular-file directory-open error.

The verified installed Tokio 1.54.1 source reads directories in 32-entry batches
and refills through blocking jobs. File types are normally cached on macOS;
this was not a per-entry metadata syscall problem. The candidate keeps HomePath
validation, then performs the finite scan and sort in one `spawn_blocking` job.
It preserves hidden-entry filtering, symlink treatment, lexical directory order,
path fields and the tested error responses. No cache, truncation or new limit is
introduced. Only `bruhs-server/src/routes/list_dir.rs` differs between the measured
baseline and candidate sources.

| Visible directories | Forward baseline / candidate p50 ms | Reverse baseline / candidate p50 ms |
| ---: | ---: | ---: |
| 0 | 0.084958 / 0.134625 | 0.128834 / 0.148041 |
| 1 | 0.159000 / 0.131958 | 0.149958 / 0.149750 |
| 10,001 | 11.232625 / 8.032375 | 11.311667 / 8.009042 |

The large response retains all 10,001 directories and 1,120,314 decoded bytes.
Its p95 changes from 21.154209 / 11.649084 ms to 8.086583 / 8.108166 ms in
forward/reverse order; p99 changes from 27.236209 / 12.004792 to
8.098750 / 8.765291 ms. Percentiles use nearest rank at the sorted position
`ceil(30 × percentile / 100)`. Thus p99 is the maximum observation, and the
forward baseline contains a large outlier. These samples do not establish stable
tail behavior or a numerical acceptance tolerance.

The control with no visible subdirectories has higher p50 by 58.5% and 14.9%; its p95 is also higher in both
orders. Unchanged read controls remain mixed: 28 of the other 120 variants have
higher p50 in both orders, 26 have higher p95 in both, and 28 have higher p99 in
both. The full observations retain equal values and all unfavorable directions.
Source work removal and the large response improvement justify preserving the
candidate for review; the repeated higher controls still need follow-up before
merge acceptance. Concurrent scans, cancellation and memory behavior remain open.
This phase does not resolve the earlier task-stream candidate's finite controls.

### New baseline costs and next investigation

These are baseline forward/reverse p50 values in milliseconds for the ordinary
1,001-contact/meeting fixtures. They compare separate storage backends, not a
store optimization. No causal disk-latency estimate follows from their difference.

| Read | Memory | SQLite | Decoded reply bytes |
| --- | ---: | ---: | ---: |
| All people | 3.109625 / 3.156666 | 4.656833 / 4.786750 | 582,677 |
| First person | 0.099083 / 0.053500 | 0.111292 / 0.107458 | 574 |
| First version page | 0.110958 / 0.070333 | 0.482833 / 0.493167 | 8,650 |
| Reverse 64-ID profiles | 0.263250 / 0.227375 | 0.421833 / 0.425291 | 36,995 |
| All meetings | 1.522416 / 1.346333 | 2.536083 / 2.511000 | 535,427 |
| First meeting | 0.099125 / 0.046625 | 0.115959 / 0.098417 | 532 |

File-child browsing still takes 19.007125 / 20.042208 ms at 10,001 visible
directories while returning the existing capped 500 entries in 27,511 bytes.
Its source scans/sorts the whole directory and lowercases names inside sort
comparisons. That is the next hypothesis to isolate, not a measured attribution
of the entire cost. Preserve complete ordering, filtering and Unicode behavior.
Store corruption/error paths, real merged chains, dense histories, concurrent
access, persistent reopen and cold caches also need coverage.

The quick workspace gate passes 2,461 Rust tests with 40 additional ignored, plus
38 site tests. Format, native/WASM Clippy, site types and benchmark checks pass;
normal server release checks pass on both sides. The baseline fixture passed
targeted tests and server Clippy before compilation. Both executables use the same
Rust toolchain and separately archived 1,253-path source snapshots. Snapshots are
workspace supersets rather than transitive dependency graphs. Full merge-gate
checks remain pending. The source/readback evidence is local; no commit, merge or
push was made, and measurement workers are stopped.

The ledger preserves 122 prior entries and adds thirteen shared partial mappings:
135 entries across 55 distinct action identities. Eighteen of the catalogue's
95 explicit HTTP identities now have mapped HTTP variants; the remaining 77 and
uncovered semantics stay open. These counts are coverage discovery, not a full
action completion percentage. Palette/slash/normal UI, physical input/paint,
providers/capture and prior native/workflow/control gaps remain in the goal.

Local evidence:

- [Complete paired observations](../target/performance-http-reads1-comparison.json) and [interactive results](../target/performance-http-reads1-comparison.html).
- [Baseline source manifest](../target/performance-http-reads1-before-manifest.json), [candidate manifest](../target/performance-http-reads1-after-manifest.json) and [Tokio source receipt](../target/performance-http-reads1-directory-source-review.json).
- [Gate receipt](../target/performance-http-reads1-after-gate-receipt.json), [candidate build receipt](../target/performance-http-reads1-after-build-receipt.json) and [resume state](../target/performance-http-reads1-work-state.json).

## Cached file-child ordering

File-child browsing now computes reusable Unicode sort keys. The 10,001-directory
HTTP fixture has 32.3% and 33.0% lower p50 in opposite execution orders, with the
same complete reply. Long-name scratch memory can increase substantially, and
unrelated controls remain mixed. The change is a local candidate; acceptance and
the full performance audit remain open.

### Ordering contract and isolated study

The old comparator lowercased both names on every same-type comparison. The
candidate uses stable `sort_by_cached_key` with directory priority and the same
whole-string Unicode lowercase operation. Equal keys retain input order, including
which records survive the existing 500-entry cutoff. Zero, one and two entries
keep the original comparator: at most one comparison offers no key reuse. The
first prototype exposed unnecessary key allocations for two entries; the revised
study confirms those allocations are removed.

Two new unit tests compare full output against the original comparator for ASCII,
expanding Unicode folds, Greek final sigma, equivalent lowercase keys, combining
forms and directory/file groups. A 1,001-entry equal-key case verifies original
membership at the 500-entry cutoff. Filtering, directory scanning, confinement,
symlinks, path fields and the output limit retain their existing implementation.
This changes the shared core file-child path used by HTTP/native callers; physical
native interaction latency is not measured here.

The final standalone study uses the exact public `FileEntry` declaration copied
byte for byte from source and imports the actual production ordering helper. It
compiles that fragment rather than the entire core crate. ASCII, Unicode,
equal-key, long-ASCII and long-Unicode data run in ordered, reversed and shuffled
input orders at 0, 1, 2, 8, 32, 500 and 10,001 entries: 105 datasets. Every sorted
vector equals the reference in full, and complete ordered JSON hashes agree.

Two uninstrumented timing processes retain 12,600 sort intervals after 2,100
excluded warmups. Input cloning, truncation, destruction, JSON/hashing and all
filesystem/HTTP work are outside those intervals. Two separate allocation
processes provide 420 observations. These totals include both methods. Counts reproduce in both method/data orders,
with zero measured live bytes at return. Peak requested bytes exclude prebuilt
input models, allocator rounding/internal realloc copies and RSS. A realloc adds
its full new requested size to the total. Tiny intervals can be zero at the
clock's resolution; they do not prove free execution or equivalence.

The table uses shuffled 10,001-entry inputs. Times are forward/reverse p50 in
milliseconds; peak is requested live scratch bytes during sorting.

| Names | Original p50 | Candidate p50 | Original peak | Candidate peak |
| --- | ---: | ---: | ---: | ---: |
| ASCII | 6.408750 / 6.378625 | 0.871791 / 0.877583 | 560,088 | 556,722 |
| Unicode | 28.874625 / 28.794167 | 1.751500 / 1.751042 | 560,104 | 636,730 |
| Equal-key mixture | 2.928542 / 2.884000 | 1.277791 / 1.290250 | 560,072 | 440,040 |
| Long ASCII | 9.467125 / 9.437542 | 1.965959 / 1.935625 | 560,512 | 2,676,934 |
| Long Unicode | 348.970417 / 349.356625 | 14.544000 / 14.601667 | 560,920 | 4,713,804 |

For shuffled ASCII, Unicode and both long-name cases, allocation calls fall from
266,535 to 10,002. Long Unicode also reduces total requested bytes from
173,027,092 to 6,870,686, despite roughly 8.4 times the peak scratch. Keeping all
lowercase keys trades allocation churn for simultaneous retention. These are
sort-only results, not a complete HTTP memory budget. The long stems are 227
ASCII bytes and 215 Unicode bytes before a directory slash. Dense long-name
HTTP, concurrent memory use and selection of only the needed 500 entries remain
important follow-up work. The first two studies remain archived as intermediate
evidence; the table and totals above use only the final 105-dataset study.

### Actual HTTP comparison and controls

The baseline is the preserved candidate executable from [Persistent and filesystem
HTTP reads](#persistent-and-filesystem-http-reads), archived as
`target/performance-http-reads1-after-binary`. The new executable keeps its exact 123-variant HTTP fixture and Rust
toolchain. Existing source changes are limited to `bruhs-core/src/files.rs`, plus
the new private helper/tests. The earlier directory worker and SSE candidates
are present on both sides and are not isolated by this comparison.

Four primary processes form two pairs: baseline/candidate forward, then
candidate/baseline reverse, also reversing case/backend/count order. Each variant
retains thirty observations after five warmups: 14,760 intervals and 2,460
excluded warmups. Two separate smoke processes use two observations after one
warmup per variant, totaling 492 excluded intervals and 246 warmups. Order and
drift are not independently isolated; samples are not pooled.

The boundary remains client execute through complete response body, using a
pooled private loopback client and two Tokio workers. Setup, request construction,
full validation, path canonicalization for fingerprints, hashing and cleanup are
excluded. All 123 canonical complete payload fingerprints, decoded lengths and
statuses agree across primary and smoke processes. Exact expected JSON, array
order, headers and cleanup pass before normalization. Only known fixture paths
are normalized; synthetic model IDs/times are fixed.

| Visible directories | Forward baseline / candidate p50 ms | Reverse baseline / candidate p50 ms |
| ---: | ---: | ---: |
| 0 | 0.135708 / 0.127542 | 0.170333 / 0.162167 |
| 1 | 0.143875 / 0.144667 | 0.174250 / 0.170750 |
| 10,001 | 19.822250 / 13.410208 | 20.068958 / 13.448500 |

Counts describe visible directories, with the same additional hidden directory,
file and two symlinks as the preceding fixture. The largest reply still contains
500 entries and 27,511 decoded bytes. In forward/reverse order, the large p95 changes from
20.252000 / 20.623042 to 14.357500 / 13.659333 ms; p99 changes from
20.428959 / 21.405459 to 14.742583 / 13.878083 ms. Percentiles use nearest rank;
with thirty samples, p99 is the maximum observation rather than a tail guarantee.

Small cases and tails remain mixed. Of the other 120 variants, 22 have higher
p50 in both orders, 25 have higher p95 in both, and 28 have higher p99 in both.
For example, SQLite all-meeting p50/p95/p99 is higher in both orders, including
a forward candidate p99 of 8.533458 ms versus 2.589750. These controls are retained
without declaring a numerical tolerance or treating unchanged source as proof of
equivalence. Earlier SSE/directory controls remain unresolved.

### Validation and checkpoint

The quick workspace gate passes 2,463 Rust tests with 40 additional ignored, and
38 site tests. Format, native/WASM Clippy, site types, benchmark checks and normal
server release check pass. Standalone study tests/Clippy also pass. The candidate
has a separately archived 1,254-path source snapshot; the baseline retains its
1,253-path archive. Both are workspace supersets rather than transitive graphs.

An initial targeted test compile failed because the disk was full. Removing only
the generated native/WASM debug incremental caches recovered about 22 GiB while
preserving performance sources, binaries and evidence. Debug checks then used
`CARGO_INCREMENTAL=0`; release measurement settings remain unchanged. The failed
compile log and recovery receipt are retained. No primary samples ran during the
failure or compilation. Full merge-gate checks remain pending; no commit, merge
or push was made. Measurement workers are stopped.

The ledger preserves 135 prior entries and adds thirteen follow-up mappings to
this shared 123-variant bundle: 148 entries across the same 55 action identities.
These remeasure existing routes and add no HTTP identity breadth. Eighteen of
95 explicit HTTP identities have mapped variants; the remaining 77, uncovered
semantics, palette/slash/normal UI, cold/concurrent/provider/capture work and
previous native/workflow/control gaps remain in the active goal.

Local evidence:

- [Paired HTTP observations](../target/performance-file-children1-comparison.json), [interactive results](../target/performance-file-children1-comparison.html) and [candidate manifest](../target/performance-file-children1-manifest.json).
- [Final sort intervals and allocations](../target/performance-file-children1-sort3-comparison.json), [study source manifest](../target/performance-file-children1-sort3-manifest.json) and [study runner](../target/performance-file-children1-sort-study3.py).
- [Gate receipt](../target/performance-file-children1-gate-receipt.json), [space recovery](../target/performance-file-children1-space-recovery.json) and [resume state](../target/performance-file-children1-work-state.json).

## Bounded file-child selection

File-child browsing now retains at most the existing 500 best candidates while
scanning. A synthetic owned-selection workload with 10,001 long-Unicode entries
reduces peak requested memory from 10,085,086 to 515,414 bytes. Actual HTTP p50
is 4.8-5.7% lower in the large-directory fixture, while reverse HTTP p99 and some
selection costs are higher. The change remains local with acceptance unresolved;
the full performance goal stays open.

### Selection contract and implementation

The baseline is the [cached ordering candidate](../docs/performance-goal-2026-11-09.md#cached-file-child-ordering),
preserved as `target/performance-file-children1-binary`. It collects every valid
entry, computes cached Unicode keys, sorts, then truncates to 500. The candidate
keeps that vector/sort path until the listing exceeds the existing cap. At the
first overflow it builds a max heap of the retained candidates; later better
entries replace the worst. Final heap order supplies the same sorted prefix.

Keys contain directory priority, the same whole-string Unicode lowercase name
and the read ordinal. The ordinal preserves stable ties and exact cutoff
membership. Its checked increment fails through the existing read-error path
instead of wrapping. Once 500 directories are retained, a later file can be
rejected without lowercasing because directory priority already excludes it.

The selector retains no more than 500 entries, plus a transient incoming entry
and conversion/output storage. It still scans every directory entry. Existing
hidden/heavy-directory filtering, symlink treatment, confinement, asynchronous
directory batching and per-entry I/O errors retain their implementation. No
new user-facing limit, cache or early scan termination is introduced.

Four selector tests cover every reference prefix of mixed Unicode/group/tie data,
stable membership at 500, late better entries, the retained-count invariant during
every push, and ordinal exhaustion. The existing file tests also pass. Only
`bruhs-core/src/files.rs` and its private `children_order.rs` helper differ from
the measured baseline.

### Owned selection CPU and memory

The final standalone study compares the exact archived cached-sort helper with
the current production selector. It uses the public `FileEntry` declaration
copied byte for byte from source, rather than compiling the entire core crate.
Both methods materialize owned models by cloning the same prebuilt seed entries
incrementally, then select the same ordered 500-entry prefix. Synthetic paths
include their filename; equal-name cases carry distinct path identities.

This window differs from the preceding sort-only study: model cloning and
discarded-model destruction are inside timing. Returned-vector destruction is
outside timing and inside the allocation window. Prebuilt seeds, JSON/hashing
and filesystem/HTTP work are excluded. Timed outputs are compared directly;
allocation-mode algorithms receive a separate full-prefix check after counting.

Five name shapes, three input orders and eight sizes (0, 1, 2, 8, 32, 500, 501
and 10,001) give 120 datasets. Two timing processes retain 14,400 intervals after
2,400 warmups. Two separate allocation processes supply 480 observations. Counts
reproduce across both method/data orders, with all measured allocations freed.
Totals include both methods. Requested peaks include owned entry copies and
selection storage, exclude seeds and allocator rounding/internal realloc copies,
and are not RSS or a full HTTP memory budget.

The table uses shuffled 10,001-entry inputs. Times are forward/reverse process
order p50 in milliseconds, not individual filesystem-operation latency.

| Names | Full-list p50 | Bounded p50 | Full-list peak bytes | Bounded peak bytes |
| --- | ---: | ---: | ---: | ---: |
| ASCII | 1.372417 / 1.374334 | 0.907084 / 0.903583 | 1,927,604 | 107,190 |
| Unicode | 2.302750 / 2.330166 | 1.567375 / 1.616292 | 2,167,628 | 119,206 |
| Equal-key mixture | 1.788208 / 1.801459 | 0.914417 / 0.910208 | 1,562,326 | 88,919 |
| Long ASCII | 2.981875 / 2.679292 | 1.568792 / 1.482334 | 8,288,240 | 425,614 |
| Long Unicode | 15.090084 / 15.685500 | 10.089709 / 10.338875 | 10,085,086 | 515,414 |

Allocation counts match the baseline for all measured datasets of at most 500
entries. Larger-input CPU is order-dependent. With reversed ASCII input, p50
rises from 0.775375 / 0.774792 to 1.646208 / 1.640208 ms. This input reversal is
separate from reversing process/method execution order.

The 501-entry transition also costs more. Shuffled ASCII p50 is
0.046209 / 0.045750 ms before and 0.075875 / 0.076000 after; peak rises from
79,273 to 107,218 bytes. Long-Unicode transition peak rises from 487,922 to
515,452 bytes. These boundary costs remain explicit alongside the large-memory
reduction. Tiny zero intervals reflect clock resolution, not free work.

An initial study runner copied the old sort executable; output-schema validation
rejected it. That failed `select1` attempt contributes no accepted selection
samples. `select2` is intermediate evidence; tables/totals here use final
`select3`, which adds the first-overflow boundary. The production helper is the
same version in the final study and HTTP executable.

### Actual HTTP comparison

The identical 123-variant read fixture runs in four optimized primary processes,
forming baseline/candidate forward and candidate/baseline reverse pairs. Reverse
also reverses case/backend/count order. Each variant retains thirty intervals
after five warmups: 14,760 intervals and 2,460 excluded warmups. Two separate
smoke processes use two observations after one warmup, totaling 492 excluded
intervals and 246 warmups. No statistical sample pooling, independently isolated drift or numerical
acceptance tolerance is established.

The boundary remains pooled private loopback client execution through complete
body receipt on two Tokio workers. Setup, request construction, full semantic
validation, fingerprint path canonicalization, hashing and cleanup are excluded.
All 123 complete canonical payload hashes, decoded lengths and statuses agree
across primary/smoke runs. Exact ordered expected JSON, headers and cleanup pass
before normalization. No provider, capture or personal store is used.

| Visible directories | Forward full list / bounded p50 ms | Reverse full list / bounded p50 ms |
| ---: | ---: | ---: |
| 0 | 0.106000 / 0.100500 | 0.183083 / 0.172083 |
| 1 | 0.152250 / 0.152250 | 0.170666 / 0.170166 |
| 10,001 | 13.231791 / 12.472833 | 13.271959 / 12.631916 |

Counts describe visible directories, with the fixture's extra hidden directory,
file and two symlinks. The largest reply remains 500 entries and 27,511 bytes.
In forward/reverse order, large p95 changes from 13.656833 / 14.282792 to
12.822833 / 12.900833 ms. P99 changes from 16.571667 / 14.550500 to
13.022541 / 20.710292 ms; the reverse candidate maximum is higher by 42.3%.
Nearest-rank p99 is the maximum of thirty observations, not a stable tail guarantee.
The single-visible-directory case has higher p95/p99 in both orders: p95 rises
7.5% / 7.7% and p99 rises 28.7% / 3.5% in forward/reverse order. Of the
other 120 variants, 26 have higher p50 in both, 19 higher p95 and 27 higher p99.
These observations and earlier control gaps remain unresolved.

### Validation and checkpoint

The quick workspace gate passes 2,465 Rust tests, with 40 additional ignored,
and 38 site tests. Format, native/WASM Clippy, site types, benchmark checks and
normal server release check pass; standalone study tests/Clippy pass too. The
1,254-path candidate snapshot and equally sized baseline archive are pinned
separately. They are workspace supersets, not transitive build graphs. Debug
checks use `CARGO_INCREMENTAL=0` to avoid recreating the cleared caches; release
settings and Rust toolchain match the baseline. Full merge-gate checks remain
pending. No commit, merge or push was made; measurement workers are stopped.

The ledger preserves 148 prior entries and adds thirteen follow-up mappings to
the shared HTTP bundle: 161 entries across the same 55 action identities. HTTP
breadth remains 18 of 95 discovered explicit identities, with 77 still unmapped.
Next measure the remaining directory-batch handoffs with cancellation/error
behavior preserved, actual dense-name/concurrent HTTP memory, the 501-entry and
reversed-input costs, and higher tails/controls. Storage semantics, normal UI,
palette/slash/providers/capture and previous native/workflow gaps remain in the
active full goal.

Local evidence:

- [Paired HTTP results](../target/performance-file-children2-comparison.json), [interactive observations](../target/performance-file-children2-comparison.html) and [candidate manifest](../target/performance-file-children2-manifest.json).
- [Final owned-selection results](../target/performance-file-children2-select3-comparison.json), [study manifest](../target/performance-file-children2-select3-manifest.json) and [study runner](../target/performance-file-children2-selection-study3.py).
- [Gate receipt](../target/performance-file-children2-gate-receipt.json), [build receipt](../target/performance-file-children2-build-receipt.json) and [resume state](../target/performance-file-children2-work-state.json).

## Contact-write HTTP coverage and name-statement reuse

Private contact-write fixtures add partial actual HTTP evidence for six more
method/path identities. SQLite saves now prepare the name-index insert once per
save. In alias-heavy fixtures, tracking and renaming p50 falls by about one third
in both process orders. Ordinary cases and unchanged memory-store controls are
mixed, so the change remains local with broader acceptance unresolved.

### Actions and state contract

The fixture uses the production router with private in-memory credentials,
connections and calendar state, plus either an in-memory meeting store or a
temporary on-disk SQLite store. It does not run application startup or call a
provider/capture daemon. Enrollment covers only the missing-contact rejection.

| Method/path | Measured branches |
| --- | --- |
| `POST /api/people` | Fresh create (201), existing-ID retry (200), blank name (400) |
| `PATCH /api/people/{id}` | Rename, empty patch/read, missing contact |
| `PATCH /api/people/{id}/tracked` | Alternating tracking state, missing contact, exhausted version |
| `PATCH /api/people/{id}/rename` | Alternating names, blank name, missing contact, exhausted version |
| `POST /api/people/{source_id}/merge_into/{target_id}` | Disjoint fresh merges, committed retry, self-merge, missing source, conflicting redirect, exhausted version |
| `POST /api/people/{id}/enroll` | Missing contact (404), before capture access |

Twenty branches run over two backends and four datasets, giving 160 variants.
Ordinary datasets have zero or 1,001 background contacts. The other datasets have
no background contacts and either 1,003 aliases on seeded target/other profiles,
or a 64 KiB Unicode observation per seeded profile. Source profiles include voice
values and overlapping ASCII/Unicode aliases. Freshly created profiles stay
minimal even in a heavy seeded fixture.

Each variant owns a fresh store. Background counts exclude three subject
profiles; committed-merge setup removes one subject. Fresh merges also seed a
disjoint pair for every request, including warmups. They shrink live cardinality;
fresh creates grow it. Primary fresh merges with no background have 73 seeded
and 38 final contacts; fresh creates have three and 38. Every row records both
counts and all decoded response lengths.

Fixed client versions of `4_000_000_000_000`, with a real-clock precondition,
exercise the existing monotonic increment exactly. Exhaustion uses `i64::MAX`.
There is no clock replacement or timestamp normalization. Every full ordered
JSON reply equals an independent model. The fixture checks committed identity/
version events, absence of extra or failed-write events, every contact and merge
redirect after each request, SQLite state through a newly opened store, security/
cache headers and temporary-directory removal. Backend-specific error/status
differences are retained. This checks one direct event subscriber, without live
SSE fan-out or UI consumption.

### Change and paired measurements

The installed, locked rusqlite 0.41.0 `Connection::execute` prepares its SQL on
every call. Contact saves called it for every display name and alias. The
candidate prepares that same insert outside the loop and executes it with the
same parameters and ordering. Payload/voice writes, name-index deletion, ASCII
folding, duplicate handling, transaction/commit and error propagation retain
their implementation. A seeded alias-heavy target has 1,004 name-insert calls;
their insert statement now has one preparation per save. Index rows are still
rewritten, including when a tracking edit leaves names unchanged.

Baseline and candidate optimized executables share the exact fixture. Their
1,256-path workspace snapshots differ only in
`bruhs-core/src/meetings/sqlite/contacts.rs`; snapshots are supersets, not
transitive build graphs. Four fresh primary processes form baseline/candidate
forward and candidate/baseline reverse pairs. Reverse also reverses backend,
dataset and case order. Each variant retains 30 intervals after five warmups:
19,200 intervals and 3,200 excluded warmups. Two smoke processes contribute
640 excluded intervals and 320 warmups.

Timing covers pooled private loopback client execution through complete body
receipt on two Tokio workers with serial requests. Setup/seeding, request
construction, exact reply/header/event/state validation, hashing, SQLite reopen
and cleanup are excluded. Validation reads between requests warm the store.
Complete response/event sequences and final-state hashes agree across all four
primary processes. Both smoke processes agree separately, since their sequences
and seed counts are shorter. Fingerprints include warmup responses/events and
are decoded-content hashes, not raw wire hashes.

The table shows alias-heavy SQLite p50 in milliseconds, with no background
contacts. Forward/reverse refer to process order.

| Branch | Baseline forward / reverse | Candidate forward / reverse |
| --- | ---: | ---: |
| Tracking toggle | 2.051708 / 2.107000 | 1.368833 / 1.356708 |
| Rename endpoint | 2.044958 / 2.115167 | 1.341875 / 1.373500 |
| Generic patch rename | 2.122958 / 2.042750 | 1.335458 / 1.372833 |
| Fresh merge | 7.697458 / 8.272125 | 7.025750 / 6.955583 |

Tracking p95 changes from 2.190542 / 2.137792 to 1.424333 / 1.420834 ms;
p99 changes from 2.448166 / 2.442166 to 1.673208 / 1.661333 ms. Rename
p95/p99 is also lower in both orders. Nearest-rank p99 is the maximum of 30
observations. The generic-patch forward baseline has p95 10.674334 and p99
11.760625 ms; its large tail difference is retained without a tail guarantee.

Ordinary SQLite rename p50 is 0.180083 / 0.159458 before and
0.170458 / 0.177709 ms after, with the reverse median 11.4% higher. Of 80
memory-store variants, 31 have higher p50 in both orders, 35 higher p95 and
34 higher p99. Of 80 SQLite variants, the counts are seven, thirteen and eight.
The missing-source memory merge in the observation-heavy fixture rises from
0.086083 / 0.071917 to 0.101166 / 0.084708 ms at p50, about 17.5% / 17.8%
higher. Memory-store implementation is unchanged. This experiment does not
separate process-to-process variation from the code change or pool samples.
Separate same-binary repeats and renewed unchanged-path controls are needed
before setting a data-backed acceptance rule. No numerical tolerance is
established here; higher controls and broader acceptance remain unresolved.

### Validation and remaining scope

The quick workspace gate passes 2,466 Rust tests, with 41 additional ignored,
and 38 site tests. Format, native/WASM Clippy, site types, benchmark checks and
normal server release check pass. A normal integration test exercises all 160
variants with two observations and smaller background count; the optimized
primary/smoke runs exercise the full workloads. Initial fixture expectations
incorrectly included internal error prefixes, and initial Clippy found two
modulus expressions. Both were corrected before either preserved binary was
built; the diagnostic logs remain available.

The ledger preserves 161 prior entries and adds six mappings to this shared
bundle, giving 167 entries and 61 distinct action identities. HTTP breadth is
24 of 95 explicit identities with partial mapped variants; 71 remain unmapped.
The enrollment mapping establishes only missing-contact rejection. No commit,
merge or push was made, and measurement workers are stopped. Full merge-gate
checks remain pending.

Continue meeting mutations and other unmapped actions, success-side enrollment/
capture, cold/concurrent storage, name-index and serialization work, allocations
and normal UI delivery. The unchanged-name index rewrite remains a cost to
measure before further changes. Existing SSE/directory/native/workflow controls,
physical input/paint, palette/slash and provider gaps remain in the full goal.

Local evidence:

- [Paired HTTP observations](../target/performance-http-contacts1-comparison.json), [interactive measurements](../target/performance-http-contacts1-comparison.html) and [runner](../target/performance-http-contacts1-run.py).
- [Baseline source/build](../target/performance-http-contacts1-before-manifest.json), [candidate source/build](../target/performance-http-contacts1-after-manifest.json) and [installed-source finding](../target/performance-http-contacts1-source-finding.json).
- [Quick gate](../target/performance-http-contacts1-after-gate-receipt.json) and [resume state](../target/performance-http-contacts1-work-state.json).

## Meeting HTTP writes and distinct-contact validation

SQLite meeting saves now validate each distinct referenced contact once and
reuse one relationship insert statement. In the repeated-contact, large-profile
fixture, title-edit HTTP p50 falls from about 51 ms to below 0.7 ms in both
orders. Distinct-contact title edits also improve. Several ordinary cases and
unchanged controls are higher, so broader acceptance remains unresolved.

### Actions and fixture state

The new fixture adds partial actual HTTP evidence for eleven method/path
identities. It runs the production router over private memory or temporary
on-disk SQLite stores, with in-memory credentials/connections/calendar and one
direct event subscriber. It does not run application startup, capture or a
provider. The stop endpoint changes the meeting record; audio-stop delivery
remains a separate action to measure.

| Method/path | Branches | Coverage |
| --- | ---: | --- |
| `POST /api/meetings` | 2 | Fresh create, blank title |
| `PATCH /api/meetings/{id}` | 3 | Title edit, empty patch, missing meeting |
| `POST /api/meetings/{id}/stop` | 2 | Existing and missing meeting |
| `PATCH /api/meetings/{id}/speakers/{speaker_id}` | 3 | Label edit, missing speaker, missing meeting |
| `POST /api/meetings/{id}/speakers/{speaker_id}/link/{person_id}` | 4 | Existing/new speaker, missing contact, missing meeting |
| `PATCH /api/meetings/{id}/skip_observations` | 2 | Alternating flag, missing meeting |
| `DELETE /api/meetings/{id}` | 2 | Disjoint fresh deletes, missing meeting |
| `DELETE /api/meetings/{id}/segments/{segment_id}` | 4 | Fresh removal, retry, missing meeting, invalid chunk |
| `GET /api/meetings/{id}/segment-removals` | 2 | Sorted removals, missing meeting |
| `GET /api/meetings/{id}/local-focus` | 2 | Native read, missing native header |
| `POST /api/meetings/{id}/local-focus` | 5 | Append, retry, meeting mismatch, missing meeting, missing native header |

Thirty-one branches across two backends and six shapes give 372 variants. Each
variant owns a fresh store. The ordinary shape starts without contact references.
The repeated shapes have 1,001 assigned action items and a linked speaker,
giving 1,002 references to one contact. The unique-assignee shape has the same
reference count but 1,001 distinct contacts. A large-person shape has one linked
contact; its observation holds 64 KiB of Unicode text. The repeated-large-person
shape combines that observation with repeated assignments. The large-summary
shape has a 64 KiB summary and no initial references.

Each row records the scale argument and actual seeded/final subject reference
and distinct-contact counts. Fresh creates grow meeting count; fresh deletes
seed a separate target per request and return to one subject meeting. New-speaker
links grow the speaker/reference list. Counts include warmups. The fixture also
seeds transcript segments, removal tombstones and synthetic local-focus metadata.
No screen content or real application activity is sampled.

### Write contract and normalization

Every full decoded response equals an independent model before normalization.
The fixture checks all seeded/created contacts and meetings, the known transcript,
removal and local-focus namespaces, committed events and absence of extra or
failed-write events after every request. It checks expected missing child state
after deletes, SQLite state through a newly opened store, response/security/cache
headers, empty 204 bodies and temporary-directory removal. Generation receipts
and speaker-link jobs are not seeded; their delete-cascade variants remain open.

Only fresh meeting IDs and the memory-store skip timestamp need normalization.
Fresh IDs must be unique valid lowercase UUID-v4 strings, with a recorded actual/
canonical witness for every request. Create start times determine exact creation
timestamps. Other edits use the existing version logic with future seed
`4_000_000_000_000` and a real-clock precondition. The memory skip implementation
writes real clock time; its value must fall within recorded request wall-clock
bounds before canonicalization. Backend-specific timestamp behavior is retained.
No clock is replaced. Canonical response/final-state hashes are decoded-content
hashes; raw decoded lengths and normalization witnesses are stored separately.

The independent model uses a generated ID or real-clock value only after those
checks, then constructs every other expected record field from fixture state and
the request. The response and stored record must match that full expected model.
Only then are the two allowed generated fields replaced for hashing.

A focused SQLite test checks duplicate speaker/action assignments, unique
relationship rows, rollback for a missing referenced contact, and full retargeting
after merge/reopen. An initial fixture reused transcript IDs across meetings;
the store rejected that completed replay. IDs were corrected before either
preserved candidate was built; the failed diagnostic remains available.

### Change and measurements

Previously, each speaker/action reference read, decoded and validated a complete
contact, then attempted an ignored-duplicate relationship insert. The candidate
tracks first-seen IDs inside the existing immediate SQLite write transaction.
It validates and inserts each distinct ID once. The insert statement is prepared
lazily after the first distinct valid contact and reused for subsequent IDs.
Zero-reference saves do not prepare it. Payload serialization, payload reference order,
first-error order, validation rules and unique relationship rows are preserved.
Tracking storage is local to one save; allocations and concurrent memory remain
unmeasured.

The first candidate suppressed duplicates only. Its separate paired study had
higher unique-assignee medians in several cases, prompting statement reuse.
All first-study artifacts remain available. The final comparison tests the
original against both changes together; it does not isolate a statement-reuse
effect by comparing numbers across studies. Samples are not pooled.

Final baseline/candidate executables share an identical fixture and Rust
toolchain. Their 1,259-path workspace snapshots differ only in
`bruhs-core/src/meetings/sqlite/records.rs`. They are supersets, not transitive
build graphs. Four primary processes form forward baseline/candidate and reverse
candidate/baseline pairs; reverse also reverses case/backend/shape order. Each
variant retains 30 intervals after five warmups, totaling 44,640 intervals and
7,440 excluded warmups. Two smoke processes add 1,488 excluded intervals and
744 warmups. The first study has the same totals, retained separately.

Timing covers pooled private loopback client execution through complete body
receipt, serially on two Tokio workers. Setup/seeding, request construction,
response/header/event/state checks, normalization/hashing, reopen and cleanup are
excluded. Validation reads between requests warm the store. All 372 canonical
response/event/final-state signatures agree across four primary processes and,
separately, both smoke processes. Signatures also agree across the two studies.
Generated IDs and wall-clock witnesses differ as expected.

The table shows SQLite title-edit p50 milliseconds in forward/reverse order.

| Shape | Baseline | Final candidate |
| --- | ---: | ---: |
| Ordinary, no initial references | 0.145792 / 0.107958 | 0.147666 / 0.108750 |
| Repeated assignees | 3.558833 / 3.549959 | 0.565375 / 0.554791 |
| Unique assignees | 4.910208 / 4.799000 | 4.033916 / 4.131708 |
| One large contact | 0.239750 / 0.172292 | 0.257125 / 0.222459 |
| Repeated large contact | 50.817500 / 50.987333 | 0.650875 / 0.632375 |
| Large summary, no initial references | 0.282959 / 0.290583 | 0.279416 / 0.301792 |

Unique-assignee title-edit p50 is 17.8% / 13.9% lower. Repeated-large-person
title-edit p95 changes from 51.470917 / 52.530750 to 0.665708 / 0.663917 ms;
p99 changes from 51.606375 / 52.861375 to 0.667125 / 0.673541 ms. Nearest-rank
p99 is the maximum of 30 samples, not a stable tail guarantee.

Ordinary title-edit medians are 1.3% / 0.7% higher. Ordinary SQLite skip-toggle
p50 rises from 0.146750 / 0.149458 to 0.163375 / 0.164833 ms, about
11.3% / 10.3% higher. Its p95 rises 35.5% / 9.5%, and p99 46.4% / 5.5%.
The one-large-contact title median is 7.2% / 29.1% higher. Of 186 unchanged
memory variants, 40 have higher p50 in both orders, 38 higher p95 and 40 higher
p99. The corresponding counts among 186 SQLite variants are 46, 49 and 48.
Process-to-process variation is not separated from the code change, and no
numerical acceptance rule is established. Same-binary repeats and renewed
unchanged-path controls are needed before broad acceptance.

### Validation and remaining scope

Both candidate quick gates pass 2,468 Rust tests, with 42 additional ignored,
and 38 site tests. Format, native/WASM Clippy, site types, benchmark checks and
normal server release checks pass. A normal HTTP integration test exercises all
372 variants with two observations and reference scale three; optimized primary/
smoke runs use scale 1,001. Debug checks use `CARGO_INCREMENTAL=0`; release
settings match the baseline.

The ledger preserves 167 prior entries and adds eleven shared-bundle mappings,
giving 178 entries and 72 distinct action identities. HTTP breadth becomes
35 of 95 explicit identities with partial mapped variants; 60 remain unmapped.
The 72 identities span all mapped catalogue surfaces; 35 are HTTP identities
and 37 are other action identities. The 95-identity denominator counts HTTP only.
Measurement workers are stopped. No commit, merge or push was made; the full
merge gate remains pending. Normal UI/paint, live event streams, meeting chat/
transcript refresh/generation, provider/capture, cold/concurrent storage,
allocation/memory and earlier control gaps remain in the active full goal.

Local evidence:

- [Final paired HTTP results](../target/performance-http-meetings2-comparison.json), [interactive observations](../target/performance-http-meetings2-comparison.html) and [runner](../target/performance-http-meetings2-run.py).
- [Original source/build](../target/performance-http-meetings1-before-manifest.json), [final source/build](../target/performance-http-meetings2-after-manifest.json) and [final gate](../target/performance-http-meetings2-after-gate-receipt.json).
- [Initial paired study](../target/performance-http-meetings1-comparison.json), [refinement decision](../target/performance-http-meetings2-refinement-decision.json), [source finding](../target/performance-http-meetings1-source-finding.json) and [resume state](../target/performance-http-meetings2-work-state.json).

## SQLite meeting-save fix landed on main

The distinct-contact validation and insert-statement reuse fix is merged in
[PR #2](https://github.com/bryantleft/bruhs-ai/pull/2), commit
`0de9416ab3708e046f08b401641ff52b9588c48a`. Local main is updated, and the
broader audit remains active. This landing contains only the SQLite save helper
and its relationship/rollback/reopen regression test.

### Landing scope and verification

The two-file patch was isolated on current main at
`4342280f1c4341014bb1cfeba9d4831768db07d7`. Both original files matched the
audit baseline exactly, and both patched files matched the measured final
candidate byte for byte. The branch commit was
`313dd9e3b10b89b4024bbc2d310ad7b8d43b2b8d`. Its complete Git tree and the
squash-merged main tree are identical: `17882b184bb12d2f3a08bddbc3f7664486c0508b`.
The merge used the expected PR head without an admin override.

The isolated full merge gate passes formatting, native/WASM Clippy, 2,317 Rust
tests (six additional ignored), benchmark self-checks, WASM build, desktop
release check, rustdoc, dependency checks and the release Trunk build. Separately,
62 site tests and the site type check pass. These counts describe the isolated
main-based checkout; earlier audit gates include more local tests and an older
site snapshot. They do not establish validation of all remaining local changes.

The first full-gate attempt passed every step except Trunk, which rejected the
inherited `NO_COLOR=1` value before building. A second complete run with
`NO_COLOR=true` passes. Both attempts and their logs are retained. Vercel checks
were successful before merge; there were no requested changes or unresolved
review threads. PR publication used the authenticated GitHub CLI because the
connected GitHub app lacked PR-write permission.

The private HTTP numbers in the [meeting-write chapter](../docs/performance-goal-2026-11-09.md#meeting-http-writes-and-distinct-contact-validation) remain evidence from the
larger audit worktree, with other unmerged performance changes present in both
sides of that comparison. They are not timings of the isolated landing branch.
The helper and test bytes match the landed change. Historical source pins,
recorded ID/clock validation, unchanged paths with higher observed latency, and
partial coverage remain intact. Cold/concurrent latency, allocation/memory and
time from user input until the next displayed frame are still open.

### Local work and next loop

Before landing, 390 dirty-file contents were recorded. Local main was advanced
through the upstream commits after temporarily stashing only the two owned,
already-merged files. All 390 file contents were checked unchanged afterward,
including those two files, and the temporary stash was removed. The subsequent
report/ledger updates are explicit additions. The isolated checkout and its
local/remote branch are removed; no build or measurement worker is running.

The evidence ledger retains all 178 measurement entries and adds a separate
landing record outside that entry count.
Coverage stays at 72 mapped identities across surfaces, including 35 of 95 HTTP
identities with partial variants. All 35 have partial coverage; this landing
marks none fully audited. Sixty HTTP identities remain unmapped. This
ships the shared save optimization while preserving the full goal and the
remaining contact/SSE/directory/native/workflow, provider/capture, palette/slash
and physical UI gaps.

Local evidence:

- [Isolation record](../target/performance-sqlite-ship1-isolation.json), [full gate receipt](../target/performance-sqlite-ship1-full-gate2-receipt.json) and [environment correction](../target/performance-sqlite-ship1-environment-correction.json).
- [Merged PR state](../target/performance-sqlite-ship1-pr-merged.json), [merge result](../target/performance-sqlite-ship1-merge-result.json), [tree/worktree integration proof](../target/performance-sqlite-ship1-integration-result.json) and [cleanup](../target/performance-sqlite-ship1-cleanup.json).

## Generation receipts and TCP transport

The network layer is part of the performance goal. Its remaining scope includes
DNS/TLS, remote and provider/proxy hops, connection reuse, transferred bytes,
stream delivery and backpressure, timeouts/retries/cancellation, concurrent
traffic and impaired links. This phase adds actual TCP evidence for four more
HTTP identities and removes a redundant SQLite receipt read when cancelling a
known generation identity. **The new fix is local; it has not been merged.**

### Scope and correctness

The fixture runs the production router with private memory or disk SQLite,
in-memory credentials/connections/calendar, and synthetic records. It never
starts capture or a model. Its 27 branches cover receipt lookup, pending lookup,
successful/failed/cancelled/pending summary replay, identity/options errors,
unknown or foreign identities, fresh pending cancellation, terminal cancellation,
fresh fencing and repeated fencing. The four method/path identities are:

- `DELETE /api/meetings/{id}/generations/{request_id}`
- `GET /api/meetings/{id}/generations`
- `GET /api/meetings/{id}/generations/{request_id}`
- `POST /api/meetings/{id}/summarize`

There are 324 variants: 27 branches × two stores × three shapes × two connection
modes. Ordinary fixtures start with 16 known receipts; history adds 1,001
cancelled receipts on one background meeting. Large-summary fixtures instead
store 64 KiB Unicode summaries in four successful receipts and their meetings.
Each primary fixture adds 105 identities, including a newly seeded pending
identity before every pending-cancel request. No live provider job is involved.

The client uses HTTP/1.1 over private IPv4 loopback. Server-observed source
sockets prove one reused connection throughout a pooled fixture, or a new
connection for every fresh request. Fresh samples include TCP establishment;
pooled retained samples reuse the warm connection. Each branch records 35
socket witnesses and decoded request/response body lengths, including warmups.

Request timing starts before client execute and records response-header arrival,
first decoded body chunk, and complete assembled body. These include handler,
transport and client processing; they do not isolate network RTT or packet-level
TTFB. Client chunk assembly is timed. Sizes exclude HTTP/TCP framing. Setup,
client/request construction, pending seeding, validation, hashing, reopen and
cleanup are excluded. Validation warms storage; requests are serial on two
Tokio workers.

Every complete decoded response, status, and selected cache/security/length
headers are checked. Every request verifies its receipt, pending identity,
unchanged meeting and absence of meeting events. All known receipts, meetings,
transcripts, removals, local context and people are checked at fixture boundaries
and after SQLite reopen. No fields are normalized. Complete response sequences
and final known state agree across versions and connection modes;
smoke comparisons are separate.

### Socket pressure and measured observations

An initial back-to-back fresh-connection sequence failed with macOS TCP connect
`AddrNotAvailable` (code 49), "Can't assign requested address." The failed
process wrote no result JSON, so its partial interval count is unknown. Three
complete earlier primary processes retain 29,160 request intervals and 4,860
warmups separately. They are not pooled into the comparison below. Source-port
pressure is the working diagnosis, not a packet-trace proof of the cause.

For the retained forward pair, loopback TIME_WAIT counts returned to or below the
observed pre-series baseline before each process. The baseline was one socket;
the host reported MSL 15,000 ms and an ephemeral range of 49,152–65,535. Readiness
checks and pre/post socket counts are retained. A later readiness guard aborted
before launching the reverse process when the system-wide count included other
apps' sockets. This is distinct from the initial socket-connect failure. A revised fixture-owned
socket guard launched no tests: build caches had disappeared, then unrelated
Cargo/Rust workers were detected. Saved binaries and source archives remain
intact. No OS or production settings were changed. The burst failure and reverse
repeat remain open; no other build worker was stopped.

Two readiness-controlled optimized processes form one forward original/
candidate pair: **19,440 retained request intervals after 3,240 warmups**, each
with three timing boundaries. Two separate smoke processes add 1,296 excluded
intervals and 648 warmups. Quantiles use nearest rank; p99 is the maximum of 30
samples. Full-body p50 observations below are milliseconds. The table includes
changed fencing paths and unchanged replay/fresh-fence context with both lower
and higher observations; it does not summarize the full population:

| SQLite branch / shape / TCP mode | Original p50 | Candidate p50 |
| --- | ---: | ---: |
| FenceSucceeded, large-summary, pooled | 0.249000 | 0.216250 |
| FenceSucceeded, large-summary, fresh | 0.340458 | 0.335458 |
| FencePending, large-summary, fresh | 0.168834 | 0.213250 |
| FenceFresh, large-summary, fresh | 0.186916 | 0.238291 |
| ReplaySucceeded, large-summary, fresh | 0.253083 | 0.273500 |
| CancelSucceeded, large-summary, pooled | 0.240333 | 0.213750 |

For large-summary `FenceSucceeded` with reuse, p95 is 0.400834
before and 0.279958 after; p99 is 0.428291 before and
0.293250 after. The interactive chart retains all variants,
boundaries and raw socket/size witnesses.

Of 162 memory variants, 91 have higher full-body p50 in this pair,
121 higher p95 and 123 higher p99. SQLite counts are
70 / 61 / 72 across its 162 variants.
The SQLite change affects known-receipt fencing; other routes provide context.
The missing reverse comparison, process variation, higher controls, allocations
and a numerical acceptance rule remain unresolved. Build-cache removal during
the phase adds environmental uncertainty. This pair does not establish causal
isolation, all-action or remote-network acceptance. Added partial route coverage
and correctness of the removed read are the checkpoint outcome; speed and merge
acceptance remain open.

### Source, validation and checkpoint

Both optimized test builds have archived 1,262-path source supersets from main
`0de9416a` plus the broader uncommitted audit work. Only the shared borrowed replay
validator and SQLite generation helper differ between capsules. Identity errors,
receipt validation and the immediate transaction remain; only Pending is saved
as Cancelled, and terminal summaries/states remain intact. No additional receipt
clone is introduced. Cargo's cached embedded commit label is historical
`b1240990` in both binaries; source manifests and copied
binary hashes identify this comparison, rather than that label.

Format checking and core/server Clippy with all targets/features pass. All
**541 core/server tests pass**, with nine ignored tests (manual timing and real-environment checks). The full merge
gate has not run for this fix. The previously merged PR #2 remains a separate
landing. No new commit, PR or merge is claimed here.

The evidence ledger grows from 178 to 182 measurement entries, with its separate
landing record preserved. Distinct mapped identities grow from 72 to 76. HTTP
mapping grows from 35 to **39 of 95**, all partial; **56 remain unmapped**. The
summarize evidence covers receipt replay and early rejection, and cancellation
measures durable fencing rather than stopping a live model. DNS/TLS, remote links,
capture/provider work, live SSE, concurrency, packet loss, wire bytes, cold
storage and physical UI delivery remain open. This phase's workers stop at this
checkpoint; unrelated builds remain untouched. The goal remains active and
incomplete.

Local evidence:

- [Comparison and raw boundaries](../target/performance-generation-tcp1-comparison.json), [interactive chart](../target/performance-generation-tcp1-comparison.html) and [source finding](../target/performance-generation-tcp1-source-finding.json).
- [Network readiness](../target/performance-generation-tcp1-network-readiness.json), [failed burst log](../target/performance-generation-tcp1-before-reverse.log) and [failed burst receipt](../target/performance-generation-tcp1-before-reverse-run.json).
- [Original source/build receipt](../target/performance-generation-tcp1-before-build-receipt.json), [candidate source/build receipt](../target/performance-generation-tcp1-after-build-receipt.json), [core/server tests](../target/performance-generation-tcp1-after-tests.log) and [Clippy](../target/performance-generation-tcp1-after-clippy.log).

## Generation TCP comparison with fixture socket readiness

The counterbalanced generation TCP repeat is complete. Fencing an already-
successful receipt with a large summary (`FenceSucceeded`) has lower full-body
medians in both orders, while many controls remain higher. **The fix remains local and speed acceptance stays open.** This pass
completes the previously missing comparison; it does not fix or retest resilience
to the earlier back-to-back fresh-connection failure.

### Retained comparison

Four optimized processes ran original/candidate forward, then candidate/original
reverse, using the same saved binaries and 1,262-path source archives as the
preceding phase. This measurement workflow made no Rust source edits or builds
and changed no OS or production settings. Before and after each process, working
source hashes matched their archived manifests and copied binary hashes matched
the build receipts. Compiler/build workers were absent at preflight/postflight.

Finishing checks later detected concurrent changes to core source paths, with
file times after the completed run log. Those edits are preserved. These measurements
identify the archived sources; current-tree acceptance needs revalidation. The
copied binaries remain usable after the earlier build caches were cleared.

The readiness check identifies socket pairs after each completed process from
the fixture's server-observed client addresses, excluding connections involving
listener addresses present before the series. It waits until those pairs are
absent from the loopback TCP table before the next process. Each process's 5,676
unique client addresses matched 5,676 postflight pairs. Every preflight found
zero remaining previously identified fixture pairs. Total loopback TIME_WAIT
counts were 1 / 1 / 1 / 2, so another app's socket did not prevent the last run.
The three inter-process checks took about 35 / 35 / 30 seconds; these waits are
outside request timing. The observed MSL was 15,000 ms. This is a harness control,
not a new application limit or a burst-performance result.

The 324 variants still cover 27 branches, two stores, three fixture shapes and
pooled/fresh TCP modes. There are **38,880 retained request intervals after 6,480
warmups**, each with header arrival, first decoded body chunk and complete
assembled body boundaries. All complete decoded response sequences, statuses,
decoded body lengths, fixture cardinalities and known final state match the
previous primary signatures across both versions, orders and connection modes.
Full store/receipt/event checks and SQLite reopen/cleanup pass. No fields are
normalized. Earlier smoke, forward-pair, burst and failed attempts remain
separate; their samples are not pooled into this repeat.

Full-body p50 below is milliseconds, forward / reverse. All rows use the SQLite
large-summary fixture shape. The table includes changed fencing branches and
unchanged-route context with favorable and unfavorable observations:

| Branch / TCP mode | Original p50 F / R | Candidate p50 F / R |
| --- | ---: | ---: |
| FenceSucceeded, pooled | 0.240917 / 0.255125 | 0.189375 / 0.203292 |
| FenceSucceeded, fresh | 0.348041 / 0.357666 | 0.285000 / 0.337667 |
| FencePending, pooled | 0.114000 / 0.111875 | 0.139958 / 0.116167 |
| FencePending, fresh | 0.185875 / 0.156958 | 0.169917 / 0.184875 |
| ReplaySucceeded, fresh | 0.271833 / 0.253542 | 0.273083 / 0.274375 |
| CancelSucceeded, pooled | 0.195167 / 0.203250 | 0.207875 / 0.213667 |

For `FenceSucceeded` with reuse, p95 is 0.271625 / 0.297750 before and
0.247125 / 0.244333 after; p99 is 0.295250 / 0.303708 before and
0.262083 / 0.248583 after. The interactive comparison includes all 324 variants,
three boundaries and raw socket/size witnesses.

Of 162 memory variants, 82 have higher full-body p50 in both orders,
78 higher p95 and 76 higher p99. SQLite counts are
73 / 77 / 83 across its 162 variants.
These counts include unchanged-route context. Pending fencing can also be
higher despite removing a receipt read. An independent variability estimate,
allocations, numerical acceptance rule and explanation of higher controls are
still missing; the selected improvement does not establish causal isolation or
that every action is faster.

### Network scope and next caller

These are warm serial IPv4 loopback HTTP/1.1 measurements, including server work
and client chunk assembly. Header/first-chunk observations are not packet-level
TTFB or isolated RTT. Sizes are decoded bodies rather than wire traffic. Fresh
samples include TCP establishment; pooled samples reuse a warm connection.
Validation/seeding/client construction/request construction/reopen/cleanup are
excluded; validation warms storage. DNS/TLS, remote/provider/proxy hops, wire
bytes, stream backpressure, errors/retries/cancellation under load, concurrency,
impaired links, cold storage and physical UI delivery remain separate work.

Source inspection found another concrete network candidate:
`ServerClient::local_context_request` constructs a dedicated reqwest client on
every call, reached by native local-focus reads and writes. The sampler polls
every two seconds but sends changes only, with at most one write outstanding.
Measure actual client construction and request delivery before changing it.
Keep its literal-loopback, proxy/redirect, timeout and redaction contracts intact;
the general client has different proxy/redirect policy. Generation-route timing
is not a measurement of this native caller. No edit to that caller is claimed.

### Validation and checkpoint

The earlier 541 passing core/server tests and Clippy checks validate the archived
candidate sources; they were not rerun after cache removal and do not validate
the later source changes. Their [test receipt](../target/performance-generation-tcp1-after-tests-receipt.json)
and [Clippy receipt](../target/performance-generation-tcp1-after-clippy-receipt.json)
record the same archived-manifest hash as the candidate build receipt. All four new optimized measurement processes pass their
full response/state/socket/reopen checks. The
new fix has not received the full merge gate and remains unmerged. Previously
landed PR #2 is separate.

The ledger adds four partial entries for the repeat: 182 becomes 186 entries,
while mapped identities remain 76, including **39 of 95 HTTP identities**, all
partial. The other **56 HTTP identities remain unmapped**. The repeat entries
retain captured-source hashes and flag the later source drift for revalidation.
No action is promoted to complete. Measurement workers have exited; the goal remains active and
incomplete.

Local evidence:

- [Repeated comparison](../target/performance-generation-tcp2-comparison.json), [interactive chart](../target/performance-generation-tcp2-comparison.html), [socket readiness](../target/performance-generation-tcp2-readiness.json) and [runner log](../target/performance-generation-tcp2-pipeline.log).
- [Original retained failure](../target/performance-generation-tcp1-before-reverse-run.json), [preceding phase verification](../target/performance-generation-tcp1-final-verification.json), [later source drift](../target/performance-generation-tcp2-live-source-drift.json) and [next native-client finding](../target/performance-generation-tcp2-network-next-finding.json).

## Native local-context client reuse

The desktop local-context caller now reuses its dedicated HTTP client after
successful initialization, including across clones. All 36 warm persistent-caller
variants have lower request medians in both orders and use one socket instead
of 35. **The fix remains local: new-instance request controls are often slower,
and first-use acceptance is unresolved.**

### Actual caller and preserved behavior

The server test crate imports the production desktop `ServerClient` source
directly. Requests pass through the production router, real private IPv4 TCP,
and private memory or disk SQLite stores. The fixture never launches GPUI, an
OS focus probe, capture, or a provider. Credentials/connections/calendar are
explicitly in memory. This extends the earlier handler-only local-focus evidence
through its actual native caller; it does not measure native frames or input.
`MeetingsHub` stores the caller in an `Arc<ServerClient>` ([hub source](../bruhs-desktop/src/views/meetings_hub.rs));
its read/sample paths clone that Arc ([local-focus source](../bruhs-desktop/src/views/meetings_hub/local_focus.rs)).
This source wiring supports persistent calls within a hub, rather than proving
the lifetime pattern of a running desktop session.

There are 72 variants: six branches × two stores × three shapes × two caller
lifetimes. Reads, fresh appends, duplicate appends, missing-meeting reads/writes,
and invalid observations are covered. A missing read returns an empty list under
the existing API; a missing append is a redacted 404 and an invalid append 400.
Ordinary fixtures seed three observations; history and wide-history seed 1,001.
Wide fields use 256 UTF-8 bytes for app name and 255 for bundle id, within the
existing limits. Fresh primary appends add 35 deterministic identities including
warmups; other cases retain their seeded state.

Previously, `local_context_request` built a new reqwest client on every call.
The candidate keeps a per-`ServerClient` asynchronous cell, shared by clones,
and initializes it only after URL validation. Successful construction is reused;
`get_or_try_init` leaves failed construction retryable under Tokio's inspected
API contract. A forced client-construction failure was not injected. The explicit
no-proxy, no-redirect and 20-second timeout settings are retained, along with the
native header and URL/status-body redaction.

The new clone contract fails on the original implementation and passes on the
candidate. Additional actual-client tests verify an unreached redirect canary,
the encoded path/native header, redacted status/decode/transport failures, and
successful retries after errors and peer connection closure. Existing URL
rejection tests also pass. Proxy environment behavior and timeout/cancellation
under load were not separately exercised in this pass.

### Boundaries and comparison

The request clock starts at the public native API call and ends at its returned
typed value or error. Successful reads include full body receipt/JSON decoding;
writes acknowledge the successful response. Status errors return without reading
their body, so their clock is not full-body delivery. Dedicated-client setup is
inside this clock. General `ServerClient::new` construction is timed separately
outside it, once for persistent callers or before every new-instance request.
The fixture sequence's first request and all warmups are retained separately.
New-instance requests are first uses of new callers even after the fixture has
warmed up. The first five fixture requests are excluded from request quantiles;
the retained new-instance quantiles therefore still describe first uses of
different callers. Constructor durations are never added to request intervals.

Model construction/checks, full stored-state/event/error checks, fingerprints,
SQLite reopen and cleanup are excluded. Validation warms storage. Requests are
serial on two Tokio workers. Result lengths are reserialized return values,
not HTTP body or wire bytes. Full ordered observations, unchanged meetings,
other known store domains and exact redacted status errors agree across both
versions, stores and caller lifetimes. SQLite reopen and cleanup pass. No fields
are normalized.

Four optimized processes form opposite-order original/candidate pairs:
**8,640 retained request intervals after 1,440 warmups**. Two separate smoke
processes add 288 excluded intervals and 144 warmups. Fixture socket readiness
is checked between processes using observed socket pairs. All 36 persistent
candidate variants use one socket for their 35 calls, including warmups; original
and new-instance variants use 35. This is not a burst-resilience result.

Request p50 below is milliseconds, forward / reverse. All rows use the SQLite
ordinary shape and include both improved persistent calls and slower controls:

| Branch / caller lifetime | Original p50 F / R | Candidate p50 F / R |
| --- | ---: | ---: |
| read, persistent | 0.116417 / 0.109250 | 0.092250 / 0.094625 |
| append-fresh, persistent | 0.202041 / 0.197250 | 0.170375 / 0.135333 |
| append-retry, persistent | 0.106292 / 0.110584 | 0.091250 / 0.052417 |
| read, new-client | 0.114791 / 0.113334 | 0.190125 / 0.182333 |
| append-fresh, new-client | 0.204500 / 0.208542 | 0.263334 / 0.264709 |
| append-retry, new-client | 0.109333 / 0.177541 | 0.112459 / 0.186458 |

Of 36 persistent variants, none has higher p50, p95 or p99 in both orders;
all 36 have lower p50 in both. Among 36 new-instance variants,
19 have higher p50 in both orders, 16 higher p95 and
13 higher p99. Quantiles use nearest rank; p99 is the maximum
of 30 samples. The fixture sequence's first request is one raw observation per process and
is excluded with warmups. General-constructor observations are a separate
component. No cold-launch distribution is inferred.
For the persistent SQLite ordinary read, those first sequence requests are
0.171333 / 0.139041 ms before and 0.345416 / 0.329875 after. General construction
is 0.003208 / 0.004792 before and 0.004250 / 0.003959 after, outside request time.
Persistent p99 is higher in three variants per order, in different variants;
ordinary read p99 rises from 0.152500 to 0.200083 ms forward, while falling from
0.157500 to 0.105917 reverse. These single-process/first observations are not
an acceptance threshold.

The first-use/control cost needs investigation before speed or merge acceptance.
The builder itself is synchronous; assess a synchronous success-only cache while
preserving initialization retry behavior. A compiler probe confirmed that
`std::sync::OnceLock::get_or_try_init` is unstable on the pinned Rust 1.95 toolchain.
No conclusion attributes the observed control cost solely to the asynchronous
cell. Allocations, retained client memory, independent variability, numerical
acceptance, error-body shapes, concurrency, proxy/timeout/cancellation, physical
UI, cold storage, DNS/TLS and remote links remain open.

### Source, checks and checkpoint

The original and candidate source supersets contain 1266 paths on main
`0de9416a` plus existing audit/provider-runtime edits. They differ only in the
desktop caller source, including its cache and policy/retry tests. The timing
fixture is identical. The setup adds a test-only desktop source import and a
uuid dev dependency; lockfile package versions are unchanged and only the
server's direct dependency list gains uuid.

Format checks, core/server Clippy with all targets/features and **564 core/server
tests pass**, with ten ignored tests. The expected original clone-test failure
is retained separately. These checks compile the actual caller module without
the native UI; the full merge gate, including the complete desktop build, has
not run for this fix. The earlier SQLite fence fix is also local; PR #2 remains
the separate prior landing.

The ledger adds two partial entries for GET/POST local focus, growing from 186
to 188 measurements. Mapped identities remain 76, including **39 of 95 HTTP
identities**, all partial, with 56 HTTP identities still unmapped. This does not
promote native UI or the whole HTTP actions to completion. Measurement workers
have exited; the broad goal remains active and incomplete.

Local evidence:

- [Comparison/raw timings](../target/performance-native-focus1-comparison.json), [interactive chart](../target/performance-native-focus1-comparison.html) and [source finding](../target/performance-native-focus1-source-finding.json).
- [Original build](../target/performance-native-focus1-before-build-receipt.json), [candidate build](../target/performance-native-focus1-after-build-receipt.json), [expected original regression](../target/performance-native-focus1-before-regression-receipt.json), [candidate tests](../target/performance-native-focus1-after-tests.log) and [Clippy](../target/performance-native-focus1-after-clippy.log).
- [Dependency delta](../target/performance-native-focus1-dependency-delta.json), [before readiness](../target/performance-native-focus1-before-readiness.json), [after readiness](../target/performance-native-focus1-after-readiness.json) and [synchronous-cell compiler probe](../target/performance-native-focus1-std-cell-probe.log).

## Native client initialization comparison

The synchronous cache experiment does not demonstrate a first-use improvement.
**Keep the
existing asynchronous cache local; do not add the experimental dependency to the
app or merge this fix yet.** Both cache variants reuse sockets and improve all
36 persistent-call medians in both orders, while new-instance controls remain
often slower. This phase changes no root production code. Merge remains deferred
until the full native gate passes and representative caller-lifetime measurements
resolve or justify the first-use/retention trade-off with independent variability.

### Comparable source and behavior

A detached checkout freezes the current source context. Three variants use the
original per-request builder, the current asynchronous cache, or a synchronous
success-only `once_cell` cache. Their 1,266-path source capsules differ only in
the production desktop `ServerClient` source. Policy/retry tests are identical.
All three share the extra dependency declarations and builder diagnostic;
package versions are unchanged. Root keeps its existing asynchronous client.

The experiment configuration differs from root in five files: workspace,
desktop and server manifests, the lockfile, and the native-focus fixture.
The extra `once_cell` declarations and diagnostic remain in the isolated
experiment. The measured asynchronous production client itself matches root.
These archived measurements are not acceptance of the complete current root
configuration. Concurrent site/vendor-license edits are preserved separately.

The actual-caller fixture remains the preceding study's six branches × two
stores × three shapes × two caller lifetimes. Each caller lifetime has 36 fixture
cases (six branches × two stores × three shapes). It uses the production router,
private IPv4 TCP and private memory/disk SQLite, without GPUI, capture, OS focus
probes or providers. Complete ordered results, exact redacted status errors,
known stored state, no emitted events and SQLite reopen/cleanup agree across
all three variants. Their result/state signatures also match the preceding
study, separately for smoke and primary runs. Timings are not pooled with that
study. No fields are normalized.

The request clock includes local URL validation, dedicated-client initialization
and successful read JSON decoding. Status errors stop at headers without
reading the error body. General `ServerClient::new` construction is measured
separately outside request time. Model checks, hashing, reopen and cleanup are
excluded; validation warms storage. Result lengths are reserialized API values,
not wire bytes. New-instance retained requests remain first uses of different
clients after fixture warmup.

### Results and decision

Six optimized primary processes run original/async/sync forward and
sync/async/original reverse: **12,960 retained request intervals after 2,160
warmups**. Three separate smoke processes add 432 excluded intervals and 216
warmups. Requests are serial on two Tokio workers. Source/binary hashes and
fixture socket readiness are checked around processes, with competing build
workers excluded. No OS or production transport settings change.

All 36 persistent variants use one socket with either cache, versus 35 original
sockets: 30 retained requests plus five warmups. All new-instance cases still
use 35. Persistent
p50 is lower in both orders for both caches. The asynchronous cache also has
lower p95 in every persistent variant in both orders; one p99 is higher forward.
The synchronous cache has one higher persistent p95 reverse, and higher p99 in
one forward and three reverse fixture cases. Against the original, neither
cache has a persistent p95 or p99 higher in both orders for the same fixture case.
Request quantiles use nearest rank; request p99 is the maximum of 30 samples.

The SQLite ordinary request p50 below is milliseconds, forward / reverse:

| Branch / caller lifetime | Original | Async cache | Sync cache |
| --- | ---: | ---: | ---: |
| read, persistent | 0.222292 / 0.114459 | 0.097125 / 0.056125 | 0.095834 / 0.056583 |
| append-fresh, persistent | 0.262625 / 0.175333 | 0.147541 / 0.115625 | 0.157208 / 0.118125 |
| append-retry, persistent | 0.200500 / 0.112125 | 0.087375 / 0.054084 | 0.089458 / 0.052750 |
| read, new-client | 0.118084 / 0.112500 | 0.169750 / 0.141334 | 0.130292 / 0.152417 |
| append-fresh, new-client | 0.195792 / 0.186875 | 0.217208 / 0.200875 | 0.198334 / 0.213250 |
| append-retry, new-client | 0.118208 / 0.172542 | 0.130625 / 0.210625 | 0.163541 / 0.160875 |

Against the original, asynchronous new-instance controls are higher in both
orders in 23/36 p50 variants, 22/36 p95 and 17/36 p99. Synchronous controls are
higher in 29/36 p50, 29/36 p95 and 23/36 p99. Comparing sync directly with async,
sync has higher medians in both orders in 19/36 persistent and 15/36 new-instance
variants; just one variant in each lifetime has lower medians in both.
This does not support replacing the existing cache initializer.

The first request of each fixture sequence remains a single observation, with
all warmups retained in raw data. For persistent SQLite ordinary reads, first
requests are 0.369834 / 0.144375 ms original, 0.341709 / 0.144375 ms async and
0.369208 / 0.152583 ms sync. These are not cold app launches. Absolute timings vary
substantially by process/order, including original persistent read medians
0.222292 versus 0.114459 ms. No independent variability estimate, numerical
acceptance or cross-study speed comparison is inferred.

The separate builder-only diagnostic measures successful equivalent builders
with empty-cell construction and client destruction outside the interval.
It has 2,000 retained samples per method/order after 200 warmups, 12,000 samples
total. No requests, router, storage or active pools are involved. Times below
are microseconds and do not attribute actual caller costs solely to the cell:

| Method / order | p50 µs | p95 µs | p99 µs |
| --- | ---: | ---: | ---: |
| direct, forward | 0.875 | 0.917 | 0.959 |
| async-cell, forward | 0.875 | 0.917 | 1.000 |
| sync-cell, forward | 0.875 | 0.917 | 1.000 |
| sync-cell, reverse | 0.875 | 0.917 | 1.000 |
| async-cell, reverse | 0.875 | 0.917 | 0.917 |
| direct, reverse | 0.834 | 0.875 | 0.958 |

Pinned Hyper source shows that retaining the client can retain its pool, and
returning an idle connection can start an idle interval. This is a source-only
follow-up candidate, not a demonstrated cause of the first-use penalty. No pool
timeout or cache policy is changed. First-use/client-retention costs, retained
memory, allocations, large or stalled error bodies, concurrent requests,
timeout/retry/cancellation, proxy environment behavior, DNS/TLS and remote or
impaired links remain open. The remaining application actions still require
their own network-layer evidence.

### Validation and checkpoint

Both cache variants pass 564 core/server tests, with 11 ignored, plus format
and all-target/all-feature Clippy checks. The original passes 563 tests with
the new clone contract skipped; that contract separately fails as expected.
Both caches pass it. Existing no-proxy/no-redirect/literal-loopback policy,
20-second timeout, redaction and error/peer-close retry behavior are retained.
Build-time failure retry follows the inspected cell contracts; it is not forced
by failure injection. Full native/merge gate acceptance remains pending.

Two setup failures are retained. Existing filesystem tests initially lacked
the checkout's `target` directory; adding an empty directory fixed the fixture.
Then copied timestamps let Cargo reuse the original test binary after switching
source variants. The clone contract caught the stale binary. Materialization
now refreshes timestamps; each release build reports a freshly compiled server
artifact. No timed results came from either failed attempt.

The ledger adds two partial HTTP entries: 188 becomes 190 measurement entries,
covering 76 distinct action identities across all surfaces. **39 of the 95
discovered HTTP identities have partial evidence**. The
other 56 remain unmapped. New entries retain all measured-source hashes and
explicitly flag the five experimental configuration differences from root.
No action is promoted to complete, no new production fix is selected, and no
merge is performed. Owned workers have exited; the goal remains active and
incomplete.

Local evidence:

- [Comparison/raw observations](../target/performance-native-focus2-comparison.json), [interactive comparison](../target/performance-native-focus2-comparison.html) and [run log](../target/performance-native-focus2-run-pipeline.log).
- [Frozen checkout provenance](../target/performance-native-focus2-isolation.json), [measured configuration differences](../target/performance-native-focus2-measured-source-vs-root.json) and [package delta](../target/performance-native-focus2-dependency-delta.json).
- [Builder-only raw data](../target/performance-native-focus2-initialization-component.json), [pool-lifetime source finding](../target/performance-native-focus2-pool-lifetime-finding.json), [fixture correction](../target/performance-native-focus2-environment-correction.json) and [timestamp correction](../target/performance-native-focus2-timestamp-correction.json).

## Capture proxy payload ownership

Capture forwarding now transfers its collected request body directly into
reqwest, removing a full payload copy. **The candidate remains local and
unmerged; latency acceptance is unresolved.** Five previously unmapped HTTP
identities gain partial two-hop transport evidence. Only one forward timing
pair is retained. Two reverse attempts passed their protocol checks but are
excluded because Cargo appeared at postflight.

### Removed work and preserved behavior

The daemon client already reuses HTTP connections. The extra work was the
server's `Bytes::to_vec`, which copied every nonempty forwarded request into
a fresh vector. The core proxy now accepts an owned `reqwest::Body`, and the
server transfers its `Bytes` directly. Pinned reqwest 0.12.28 source stores
those bytes without adding a payload copy. The source-derived avoided copy is
the request length, up to the existing 1 MiB input cap. This is not a measured
allocation count or peak-memory result; ordinary accepted bodies are small.

No dependencies or limits change. Token files are reread on every call for
daemon restart rotation. Existing timeout, proxy/redirect, connection reuse,
header/status/body and error-mapping policies remain in place. Only the core
proxy argument/test and server body conversion differ between the frozen
implementations. The two selected root forwarding files match the measured candidate.
Other desktop-source drift remains separate.

### Two-hop protocol evidence

The fixture exercises the actual production server router and core capture
client through incoming-client/server and server/daemon private IPv4 TCP hops.
An owned daemon model records full requests and supplies deterministic replies.
Authorization reads a private synthetic token file. AppState's secrets,
connections, calendar and meetings are explicitly isolated/in-memory; no app
background workers, real capture hardware, provider or browser engine starts.
An explicit daemon injection helper and the timing fixture are common to both
implementations.

Five identities are covered: capture status, start, stop and GET/POST activity
monitor. Twenty-six scenarios × reused/fresh incoming transports give 52 cases.
The four proxy calls each exercise ordinary, refused-token, 64 KiB opaque and
oversized replies. Four additional cases forward 64 KiB/1 MiB rejected payloads
to a modeled 422 response. An input over 1 MiB is rejected before the daemon.
Status covers idle/recording, malformed JSON, token refusal and a reserved
non-listening endpoint. Large rejected inputs test transport pass-through,
not the real daemon's payload validator. Opaque replies contain invalid UTF-8.

Each request checks complete forwarded bytes, method/path/content type, the
daemon token instead of the incoming credential, full returned bytes or exact
JSON, status/security headers, unchanged empty meeting/people state and no
events. Both versions agree on all ordered response/forwarded-byte fingerprints
across incoming transports, separately for smoke and primary runs. No fields
are normalized. A normal test rotates the token after the first request and
checks the next request uses the replacement. Timed runs use a stable token.

### Timing evidence and excluded runs

The clock runs from incoming reqwest `execute` through complete response bytes.
It includes both TCP hops, token-file reads and server/daemon body buffering.
Request building, fresh incoming-client construction, full checks/hashing and
cleanup are outside. First requests, all warmups and fresh-client construction
durations are retained separately. Requests are serial on two Tokio workers.
Payload lengths are HTTP body bytes, not complete wire traffic.

Each case has 30 retained requests after five warmups, per implementation.
The retained forward-only pair contains **3,120 intervals after 520 warmups**.
Two separate smoke runs add 208 excluded intervals and 104 warmups. Both primary
processes have no build workers observed at preflight/postflight, plus
source/binary hash checks and fixture socket readiness. Build-worker observations
are checkpoints, not continuous host-activity monitoring. This is a single-order comparison,
without independent variability or numerical acceptance.

Both attempted candidate reverse processes completed all transport checks.
Each then found a Cargo process at postflight. Its command/lock-wait state was
not captured while live, so compilation contention cannot be ruled out. Their
combined 3,120 request intervals and 520 warmups remain archived and excluded
from the comparison. Another resume preflight found Cargo/rustc. No original
reverse process ran. A complete reverse pair remains pending a quiet host.

Normal complete-body upstream cases reuse one server-to-daemon connection for
35 requests per scenario/process (30 retained plus five warmups), including refused-token and
malformed-status replies. Fresh incoming clients use 35 sockets, while normal
pooled incoming clients use one. Oversized-input and oversized-reply peer
counts remain explicit. Body ownership introduces no new pooling policy.

The reserved non-listening status endpoint takes about two seconds per call
at the existing connect deadline on this host, accounting for most run time.
That does not establish every refusal, header/body stall or remote failure.
The implementation
timeout policies are unchanged; the fixture does not validate every timeout path.

Forward request p50 is milliseconds:

| Scenario / incoming transport | Original copy | Transferred body | Change |
| --- | ---: | ---: | ---: |
| start/ordinary/pooled | 0.192459 | 0.176417 | -8.34% |
| monitor-command/ordinary/pooled | 0.083792 | 0.106208 | +26.75% |
| start/rejected-1048576/pooled | 0.648750 | 0.647000 | -0.27% |
| monitor-command/rejected-1048576/pooled | 0.326875 | 0.470125 | +43.82% |
| status/idle/pooled | 0.084708 | 0.086458 | +2.07% |
| start/ordinary/fresh | 0.403500 | 0.360083 | -10.76% |
| monitor-command/ordinary/fresh | 0.145625 | 0.183750 | +26.18% |
| start/rejected-1048576/fresh | 0.462166 | 0.439458 | -4.91% |
| monitor-command/rejected-1048576/fresh | 0.433709 | 0.467375 | +7.76% |
| status/idle/fresh | 0.144542 | 0.168333 | +16.46% |

The 12 affected cases per mode are six start and six monitor-command scenarios.
The 14 controls are four stop, four monitor GET, five status scenarios and
oversized start input. Among the affected cases, pooled medians are lower
in four and higher in eight; fresh medians are lower in three and higher in
nine. Of 14 controls per mode, 13 medians are higher in each. Controls moving
in the same direction prevent attributing these latency differences to body
ownership alone. Tails and first observations remain in the comparison;
request p99 is the maximum of 30 samples. Source verifies less copying,
while the timing evidence does not establish a general speed improvement.

### Validation and checkpoint

Both frozen implementations pass **565 core/server tests**, with 11 ignored,
plus format and all-target/all-feature Clippy. Every retained release server
artifact is freshly compiled. The first full-suite attempt lacked an empty
checkout `target` directory required by two existing filesystem fixtures.
Setup/build now enforce it; that failed attempt contributes no timings.

The source supersets contain 1,267 paths and differ only in the two forwarding
files, including the adapted existing test. No lockfile/package changes occur.
Concurrent desktop UI/vendor-license edits are preserved and retained as
captured-source drift. The snapshots prove the frozen comparison, not full
current desktop acceptance. The narrow local patch is ready for further review;
latency acceptance and the full merge gate remain pending. Latency acceptance
requires the reverse pair and independent repeats that explain ordinary/control
and tail variation, with budgets grounded in actual workflows. Before merging,
run `scripts/gate.sh` on the intended merge tree, including desktop checks.
Real UI/capture acceptance remains wider audit work.

The ledger grows from 190 to 195 partial measurement entries and from 76 to 81
distinct action identities across all surfaces. **44 of 95 HTTP identities
have partial evidence**, leaving 51 unmapped. No action is complete. Real
capture/recording/provider/UI work, allocations/memory, cold storage, configured
remote/proxy links, DNS/TLS, wire traffic, streaming/backpressure, concurrent
and broader impaired-link behavior remain open. Owned workers have exited;
the broad goal remains active and incomplete.

Local evidence:

- [Retained forward-only comparison](../target/performance-capture-proxy1-comparison.json), [interactive comparison](../target/performance-capture-proxy1-comparison.html) and [timing disposition](../target/performance-capture-proxy1-timing-disposition.json).
- [Source finding](../target/performance-capture-proxy1-source-finding.json), [narrow patch](../target/performance-capture-proxy1-request-body.patch), [snapshot](../target/performance-capture-proxy1-isolation.json) and [selected local change](../target/performance-capture-proxy1-selection.json).

## Capture payload copy: two-round transport repeat

> Later readiness qualification: this phase's absent-build-worker checkpoints
> are unproven because macOS truncated the process names used by its matcher.
> Its raw timings and separate source/correctness/lock receipts remain. See
> [the SSE audit correction](#capture-event-stream-frame-copies-and-delivery-outcomes)
> and the five affected ledger records before interpreting the timing results.

The fresh repeat confirms identical transport behavior and some repeatable
median improvements with reused incoming connections. **Fresh-connection
results and tails remain mixed, so there is no general latency acceptance.**
The previously selected payload-copy removal is unchanged. Its isolated
main-tree validation is tracked separately; no wider audit work is declared
complete by this comparison.

### Correctness retained, timing protocol revised

The full correctness test still covers all 26 scenarios in both incoming modes,
including token rotation and the reserved non-listening endpoint. The latency
fixture selects the other 25 scenarios × two incoming transports, or 50 cases.
Only `status/unreachable` is omitted from this latency cohort. Its unchanged
deadline already accounts for about 140 seconds per process in the preceding
phase; that raw evidence and its two excluded reverse attempts remain intact.
The new cohort does not revalidate or promote those excluded timings.

The same revised fixture is compiled for both body-ownership implementations.
Here pooled means a reused incoming client. Forward/reverse changes the order
of scenarios and incoming modes, not HTTP request direction. Each timed call still passes through the actual production router and core
capture client over two private IPv4 TCP hops. An owned daemon model records
full forwarded bytes and supplies fixed replies. The normal test retains the
unavailable deadline; no production timeout or behavior changes.

The response, forwarded-byte and state checks from the preceding phase remain.
All 25 timed scenario signatures agree across both implementations, incoming
modes and rounds, separately smoke/primary. They also match the same scenarios
in the preceding dataset. No fields are normalized and no timings are pooled
across phases. The model does not exercise real capture hardware, providers or
browser rendering.

### Four paired process comparisons

Round A runs original-forward, candidate-forward, candidate-reverse and
original-reverse. Round B reverses the version order within each fixture order.
All eight primary processes are retained: **12,000 request intervals after
2,000 warmups**. Two separate smoke processes add 200 excluded intervals and
100 warmups. Primary cases have 30 retained requests after five warmups per process.
Smoke cases have two retained requests after one warmup per process.
No attempt is excluded in this new repeat.

The driver holds shared build-profile locks only during each measurement
process and its postflight snapshot, then releases them for readiness waits
and result validation. It records live build commands and would exclude an
attempt if build activity appeared at postflight. No build workers were observed
at the retained preflight/postflight checkpoints; these are observations at
checkpoints, not continuous monitoring of every host activity. Identified
fixture socket pairs clear between processes. No OS setting changes.

Primary process elapsed times range from 470.39 to 1140.82 ms.
This is fixture runtime: repeated waits for an unchanged deadline are outside
this latency cohort. Correctness/deadline coverage remains in the full test.

The clock still measures incoming reqwest `execute` through complete response
bytes, including both TCP hops, token-file reads, core status decoding and
body buffering. Request building, incoming-client construction, checks/hashes
and cleanup are excluded. Raw first requests, all warmups and fresh-client
construction times remain available. Per primary scenario/process, normal reused incoming connections use one
socket for 35 calls; fresh incoming connections use 35. The independent
server-to-daemon hop reuses one socket when the reply is fully consumed. Oversized
input/reply peer counts stay explicit. Payload lengths are HTTP bodies, not
complete wire traffic.

### Results and limits

Counts below identify cases lower or higher in **all four matched process
pairs**. Twelve cases per incoming mode have a request copy removed; thirteen
controls have no forwarded body copy. The removed-copy group is six start and
six monitor-command cases. The unchanged deadline is separate from these counts.

| Incoming / group | Cases | p50 lower / higher all four | p95 lower / higher all four | p99 lower / higher all four |
| --- | ---: | ---: | ---: | ---: |
| pooled, copy-removed | 12 | 4 / 0 | 2 / 0 | 2 / 0 |
| pooled, control | 13 | 4 / 0 | 1 / 0 | 2 / 0 |
| fresh, copy-removed | 12 | 0 / 0 | 0 / 0 | 0 / 0 |
| fresh, control | 13 | 0 / 1 | 0 / 0 | 1 / 0 |

Selected p50 observations below are milliseconds, original / candidate:

| Scenario / incoming transport | A forward | A reverse | B forward | B reverse |
| --- | ---: | ---: | ---: | ---: |
| start/ordinary/pooled | 0.128375 / 0.209708 | 0.127958 / 0.088625 | 0.218834 / 0.176916 | 0.263542 / 0.122125 |
| monitor-command/ordinary/pooled | 0.088250 / 0.086292 | 0.091875 / 0.087875 | 0.111250 / 0.107667 | 0.152208 / 0.099167 |
| monitor-command/ordinary/fresh | 0.166209 / 0.156875 | 0.155875 / 0.157917 | 0.170000 / 0.186209 | 0.212375 / 0.381500 |
| start/rejected-1048576/pooled | 0.689334 / 0.633291 | 0.578459 / 0.329042 | 0.649500 / 0.508791 | 0.444791 / 0.586625 |
| monitor-command/rejected-1048576/pooled | 0.511959 / 0.323792 | 0.517208 / 0.349791 | 0.552375 / 0.485875 | 1.064792 / 0.591333 |
| status/idle/pooled | 0.108750 / 0.086625 | 0.144000 / 0.086542 | 0.146042 / 0.144917 | 0.299500 / 0.095084 |

The 1 MiB monitor-command case with reused incoming connections has lower
medians in every pair, while the 1 MiB start case rises in the last pair.
Controls also show sizable shifts. Source inspection independently proves removal of the payload copy. The
observations show lower medians in some cases, with controls also shifting;
they do not attribute every latency change to ownership. Fresh incoming cases provide no copy-affected median that
is consistently lower or higher across all four pairs. The fresh monitor-read/refused control has
higher medians in all four pairs. All per-pair controls and tails are retained.
Quantiles use nearest rank; request p99 is the maximum of 30 samples.

No numerical budget is inferred from these process observations. Allocation
counts, retained memory, independent host/workload variation, real UI/capture
work, cold storage, configured proxy/remote/DNS/TLS hops, wire traffic,
streaming/backpressure, concurrent requests and broader impaired links remain
open. Source confirms removal of one full nonempty request-payload copy, up to
the existing 1 MiB cap; this is not an allocation/peak-memory measurement.

### Source and checkpoint

Each source superset contains 1268 paths; they differ only in
the two forwarding files, including the existing core test adaptation. Both
versions each pass **565 core/server tests**, including the full 52-case
correctness test, with 11 ignored, plus format and
all-target/all-feature Clippy. Both retain the 52-case correctness test.
Dependency/lock versions and production transport policies are unchanged.
Concurrent root edits are preserved and captured-source drift remains explicit.

The narrow patch's original files match remote main at `0de9416a`. A separate
clean main checkout contains only those two source changes for full
`scripts/gate.sh` validation. Its exact expected tree is
`6a31da472666552b9e865ec927fd4132afba289c`. The benchmark fixture and unrelated
desktop edits are outside that candidate. Only the two selected forwarding
files match between the measured WIP candidate and clean merge candidate; the
565-test results and timing observations apply to the archived WIP context.
The clean tree requires its own full merge gate. Its current status is in the
[gate state](../target/performance-capture-ship1-work-state.json), with the
[gate receipt](../target/performance-capture-ship1-gate-receipt.json) written
when that command completes. At this checkpoint, the full gate and supplemental
site checks passed on that clean tree (2,317 Rust tests,62site tests,typecheck).
Before merging, require the exact intended tree to pass scripts/gate.sh and
the supplemental site tests/typecheck. The patch remains selected because
ownership removes demonstrated excess work while preserving transport behavior;
these mixed timings are not a uniform latency claim.

Cumulatively, the ledger adds five repeat entries, growing from 195 to 200 partial
measurement entries. These records reference the eight primary processes;
they are not process counts. No new identities are added: the total remains 81, including **44 of 95 HTTP
identities**, all partial, with 51 unmapped. No action is promoted to complete.
Measurement workers have exited; the broad goal remains active and incomplete.

Local evidence:

- [Repeated comparison](../target/performance-capture-proxy2-comparison.json), [interactive comparison](../target/performance-capture-proxy2-comparison.html), [selected runs](../target/performance-capture-proxy2-selected-runs.json) and [run log](../target/performance-capture-proxy2-run-pipeline.log).
- [Protocol preparation](../target/performance-capture-proxy2-preparation.json), [source snapshot](../target/performance-capture-proxy2-isolation.json), [readiness](../target/performance-capture-proxy2-readiness.json) and [narrow merge-candidate setup](../target/performance-capture-ship1-setup.json).

## Capture proxy copy removal landed

[PR #3](https://github.com/bryantleft/bruhs-ai/pull/3) is merged into main at `2d1a29a60e596e3ba17e6efb66ad15f2d998df5a`.
Only `bruhs-core/src/capture_client/mod.rs` and
`bruhs-server/src/routes/capture_proxy.rs` shipped. They remove the request-body
copy and adapt the existing core proxy test. Benchmark helpers and unrelated
desktop/audit work remain outside this landing.

The merged tree `6a31da472666552b9e865ec927fd4132afba289c` exactly matches the tested clean-main candidate.
The full gate passed with 2,317 Rust tests, zero failures and six ignored, plus
native/Wasm Clippy, Wasm and desktop checks, rustdoc, dependency checks and Trunk
release build. Supplemental 62 site tests and typecheck passed. This is source
integration evidence; real capture hardware, UI latency and provider throughput
are still wider audit work.

Local main was fast-forwarded after verifying the landed file set. Immediately
before and after integration, hashes matched for all 1,899 captured tracked
files and non-ignored untracked paths. This covers that snapshot interval,
not ignored files or later edits. The selected patch was already present
locally; integration committed it without changing working bytes. The index
outside the two landed paths was preserved. The task's scoped stash was removed; pre-existing
stashes were preserved. The two landed files are now clean on local main.

The ledger retains all 200 measurement entries and adds a second separate
code-landing record. Evidence remains partial for all 81 mapped action identities, including
44 of the 95 discovered HTTP identities. The two-round timing observations stay separate from exact-main
validation and do not establish uniform latency acceptance. The broad goal,
including the remaining 51 unmapped HTTP identities and other network/UI flows,
remains active and incomplete.

Local evidence: [merge record](../target/performance-capture-ship1-merged.json),
[exact-tree checks](../target/performance-capture-ship1-verified.json),
[root integration](../target/performance-capture-ship1-root-integration.json) and
[repeated transport observations](../target/performance-capture-proxy2-comparison.json).

## Capture event stream: frame copies and delivery outcomes

The local candidate removes the temporary `Vec<u8>` copy of each decoded SSE
frame and repeated movement of the remaining input bytes. Owned payload strings
and the diagnostic output vector still allocate. It borrows complete frames and compacts
only when a new input chunk arrives. Large batches improve, but one-byte
chunks regress. **Keep this candidate unmerged.** The next candidate must remove
the repeatable one-byte regression in paired measurements, preserve the batched
gain and event/reconciliation contract, and pass allocation/memory checks and a
clean-main gate. No workflow latency budget has been established.
No size limit, broadcast capacity, retry, timeout, proxy or redirect policy changed.

The frozen before/after Rust source supersets contain **1,269 paths each** and
differ only in `bruhs-server/src/routes/meetings_stream/daemon.rs`, including
its adapted existing test helper. The same test-only parser/TCP fixture and
shared actual-router host run in both versions. Each passes **567 core/server
tests, zero failures, 12 ignored**, selected-file formatting and all-target,
all-feature Clippy. This includes every two-chunk split in a mixed LF/CRLF,
UTF-8, comment, invalid-frame and unfinished-tail fixture, plus the event-or-
reconciliation TCP contract. Existing retry/EOF/cancellation/fan-out tests stay
in the suite. Whole-workspace formatting failed on unrelated pending desktop/web
edits; its log remains. No full clean-main gate was run for this candidate.

Four matched before/after process pairs span two rounds and forward/reverse
fixture order. Round B reverses version order. Each case/process retains
20 intervals after three warmups; nearest-rank p99 is the maximum of 20.
**2,880 retained intervals and 432 excluded warmups** cover 12 parser cases
and six TCP cases in each of eight primary processes. Separate smoke data
contains 72 excluded intervals and 36 warmups. All ten release attempts pass and none is discarded.
The two smoke processes stay outside the primary comparison. The raw files retain every interval and delivery outcome.

Parser cases use 1/32/512/8,192 events with 1-byte, 257-byte and whole-input
chunks. The timer covers decoding into complete owned payload vectors;
input construction, equality checks, hashing and payload destruction are outside.
Every payload and incomplete tail matches. The one-event input is only 136
bytes, so configured 257-byte and whole-input cases have identical effective
chunk boundaries and duplicate raw labels. Analysis preserves both schedule
entries using source-schedule/recorded-order keys, checks 18 unique keys per
process and verifies the total interval counts. No raw timing is changed.

| Parser workload | Before p50, ms (A-F / A-R / B-F / B-R) | After p50, ms |
|---|---|---|
| 8,192 events, whole input | 75.507 / 77.227 / 73.507 / 78.245 | 1.927 / 1.978 / 1.944 / 1.992 |
| 8,192 events, one-byte chunks | 6.142 / 6.199 / 6.031 / 6.248 | 6.461 / 6.480 / 6.425 / 6.521 |

Eight of 12 parser cases have lower p50 in all four pairs. The 32/512/8,192-
event one-byte cases have higher p50 in all four pairs, about **3–9%** higher.
The one-event one-byte case is mixed. Full p50/p95/p99 values, warmups and
startup observations remain in the comparison artifact. Removing frame copies
is source evidence; allocations, peak memory and normal UI impact were not measured.

The TCP clock starts at sending a prepared batch to the synthetic producer
and ends after the reqwest client frames the last known event. It includes
two private IPv4 HTTP/TCP hops, production daemon parsing/pump/broadcast,
actual `/api/meetings/stream` routing and SSE serialization. Request preparation,
producer chunk copies, byte checks and hashing are outside. Each process has
one authenticated daemon request and one incoming subscription/socket. Producer
chunk boundaries do not assert TCP/reqwest chunk boundaries. Byte counts are
payload/framing bytes, not complete wire traffic.

All 1/16-event batches arrive in full byte-for-byte equality. None of their
four cases has consistently lower or higher p50/p95/p99 across all four pairs.
Both 128-event workloads remain, with their timings excluded from speed
comparisons before release measurements because of the existing **64-event
broadcast queue**. All fragmented 128-event timed batches arrive completely.
Every whole-batch timed case skips 64 events and delivers a reconciliation notice
plus the final 64 events, identically before and after. Per version, 480 timed
batches contain 23,200 input events: 18,080 delivered, 5,120 skipped and 80
reconciliation notices. Every delivered frame, order and required gap notice
is checked; per-sample positions/byte counts/digests are preserved. Private
meeting/people stores stay empty. This does not measure client reconciliation
fetches or native/browser delivery and backpressure under other loads.

**Readiness evidence correction.** Current macOS `ps` output truncates `comm`
to `~/` for observed Cargo/rustc processes. The previous
`performance-capture-proxy2-run.py` matched only that basename, so its
absent-build-worker checkpoints are unproven. Its raw timings, source/signature/
test evidence and recorded locks remain; it already made no uniform latency
acceptance. Correction `capture-proxy2-process-readiness-qualification-2026-11-12` annotates
the five records in bundle `capture-proxy-repeat2-2026-11-12`: GET capture status,
POST capture start/stop and GET/POST activity-monitor. The separately merged
body-copy change and its clean-main correctness gate are unaffected.
This finding also applies to the process-name predicate in that phase's final
checker; live terminal process receipts remain separate evidence.

The new runner matches full executable paths from `args` and records reported
`comm` separately. Two owned native Cargo probes, debug and release, demonstrably
block on held profile locks and are detected; both terminate before measurements.
Each measured process holds four shared profile locks only for execution and
postflight observation, releases them before readiness waits/validation, and
checks identified prior fixture sockets clear. Retained preflight/postflight
build checks are clear. These are checkpoints, not continuous whole-host monitoring.
No operating-system tuning was applied.

The three initial debug fixture failures (whitespace oracle, malformed synthetic
token and an incorrect loss-free 128-event expectation) remain archived with
corrections. They precede the common final fixture and all retained release runs.
The audit now has **201 variant evidence records across 82 distinct catalogue
identities**, with **45 of 95 HTTP identities mapped and 50 unmapped**. Mapped
means at least one partial record, not complete action coverage. This phase adds
`GET /api/meetings/stream`, evidence ID
`sse-frames1-get--api-meetings-stream-2026-11-12`. No whole action is accepted. Next work
must address the one-byte regression, independently inspect allocations/memory,
normal delivery/reconciliation costs and real capture/UI/remote/DNS/TLS paths,
then pass a clean-main gate before shipping this candidate. Other application
actions remain in the full goal.

Artifacts: `target/performance-sse-frames1-comparison.json`, `-comparison.html`,
`-selected-runs.json`, `-analysis-receipt.json`, `-process-observation.json`,
`-lock-probe.json`, `-label-correction.json`, `-preparation.json` and the pinned
source/binary/check/run receipts. Historical capture evidence stays intact.

## Capture event stream: forward scanning and allocation probes

The revised scanner removes the earlier one-byte computation regression and
retains the large-batch gain. **Keep it unmerged while the TCP tail observation
is investigated with matching test harnesses.** The 16-event whole-batch maximum
is higher in all four native pairs, but the candidate binary also registers an
extra test. Correctness and the full merge gate pass; those results do not
establish whole-action speed acceptance.

The parser checks a delimiter when its final LF arrives, looking back only within
the unconsumed frame. It scans new bytes rather than rescanning an overlapping
three-byte suffix. The earlier borrowed-frame cursor still avoids a temporary
`Vec<u8>` copy per complete frame and shifts an unfinished tail only before a
new chunk. No size limit, queue capacity, token/auth, pooling, proxy, redirect,
timeout or retry policy changes.

**Exact source computation.** Three production parser substrings (committed
main, the prior cursor prototype and the revised scanner) compile with one common
standalone driver at Rust O3/LTO/codegen-units1. No source edits occur inside the
extracted substrings. The original full file matches committed main. Every run
checks 5,461 short binary CR/LF/input patterns under four chunk choices, including
repeated polls without new input: 21,844 check executions, not unique action
variants. Every measured decode then checks complete expected payloads and the
unfinished-tail length without normalization.

The 34 uniquely labelled configurations cover single-/multi-line data at
1/32/512/8,192 events, 1-byte/257-byte/whole-input chunks, a 64 KiB line,
1,024 data lines, and dense comment/empty-data frames. Some configured chunk
caps exceed the input size; these physically equivalent rows remain separate
scheduled observations. Invalid UTF-8 frames and an unfinished tail remain in
each fixture.

Four three-version groups counterbalance original/revised version order and
forward/reverse case order. The prior prototype remains the middle diagnostic.
Each case/process has 32 retained samples after five warmups: **13,056 primary
intervals and 2,040 excluded warmups**. Three separate smoke processes retain
204 excluded intervals and 102 warmups. Nearest-rank p99 is the maximum of 32;
process quantiles are not pooled. All 21 standalone timing/resource processes
pass and none is discarded.

| Parser workload | Original p50 ms, four groups | Revised p50 ms |
|---|---|---|
| 8,192 multi-line events, one-byte chunks | 5.830 / 5.845 / 5.731 / 5.833 | 3.506 / 3.501 / 3.455 / 3.483 |
| 8,192 multi-line events, whole input | 73.313 / 73.871 / 73.291 / 74.974 | 1.296 / 1.274 / 1.247 / 1.251 |

All 34 configurations have lower p50 than original in all four groups; 31 have
lower p95 and 27 lower p99 in all four. None has consistently higher p50/p95/p99.
Against the prior prototype, 33 have lower p50 in all four groups, one is mixed,
and none is consistently higher. These are isolated decode results, not normal
UI or end-to-end action budgets.
Individual isolated tails still increase: p95 for `empty-data-8192/1` in b-forward is 0.920209 to 2.244417 ms (+143.9%); p99 for `multi-1/whole` in a-forward is 0.000542 to 0.004291 ms (+691.7%). These mixed observations remain in the comparison and are not a uniform-tail acceptance.


**Allocation probes are separate binaries.** Timing builds contain no counting
allocator. Two process orders with three serial probes per case/version produce
**612 allocation observations**; all six observations agree per case/version.
The probe ends while returned outputs remain live, and retained requested bytes
match their `Vec`/`String` capacities. No configured case increases allocation/
reallocation calls, total requested bytes, logical requested peak or retained
requested output bytes against original.

For 8,192 multi-line events in one input, allocation calls drop **25,061 to
16,386**, with 12 reallocations in both versions. Total requested bytes drop
3,827,621 to 2,944,854. Logical requested peak changes only from 2,027,619 to
2,027,510 bytes; returned output remains 1,144,660 bytes. The declared
`FrameBuffer` type grows from **32 to 40 bytes** on this target. These measurements
do not establish physical allocator peaks, async-future/stack high-water or RSS.
The first cursor and revised scanner have identical recorded heap requests.

**Actual TCP stream.** A clean-main source checkout plus a common private fixture
and temporary existing-version SHA2 dev/lock edge yields 1,018 Rust/manifest paths
per version. Only `daemon.rs` differs. The before/after suites pass **526/527
core/server tests**, zero failures and five ignored, with whole-workspace
formatting and focused all-target/all-feature Clippy passing. The extra candidate
test is the new binary-framing oracle, which is the registration confound to remove.

Four matched TCP pairs span forward/reverse fixture order and reversed version
order. The first two precede the gate; two more repeat the initial higher tails
afterward using the unchanged archived test binaries and the same cwd. The live
checkout then contains the shipping tree without the measurement overlay;
archived source/binary hashes are checked. The compiled fixture reads private
temporary token/producer data and memory stores, rather than source/build files.
No quantiles are pooled across those epochs.

Six cases/process use 1/16/128 events with 13-byte/whole producer chunks,
32 retained samples and five warmups: **1,536 primary intervals and 240 warmups**.
Separate smoke data contains 24 excluded intervals and 12 warmups. The clock is
producer-channel send through framing the final known event over two private
IPv4 TCP hops, production daemon parsing/pump/broadcast and actual HTTP routing.
Inputs, producer chunk copies, checks/hashing and cleanup are outside. Producer
chunks do not assert TCP/reqwest chunk boundaries; counts are payload/framing,
not complete wire traffic. Each process observes one authenticated upstream
request/socket and one incoming subscription/socket. Stores remain empty.

Every 1/16-event frame arrives in full byte equality. Fragmented 128-event batches
arrive completely. Every whole 128-event timed batch delivers a reconciliation
notice and the final 64 events, identically before/after under the unchanged
64-event queue contract. Per version across both epochs: 768 timed batches,
37,120 input events, 28,928 delivered, 8,192 skipped and 128 reconciliation
notices. Positions, full observed-byte digests and notice counts stay in the raw
records. The 128-event timings are delivery diagnostics, not speed comparisons.

Both 16-event cases have lower p50 in all four pairs. The one-event medians vary.
The 16-event whole-batch maximum-of-32 observation remains higher in all four:

| Native pair | Before max32 ms | After max32 ms | Difference |
|---|---|---|---|
| A | 0.274 | 0.325 | +18.7% |
| B | 0.253 | 0.313 | +23.5% |
| C | 0.266 | 0.299 | +12.1% |
| D | 0.281 | 0.345 | +22.7% |

Other tail directions vary. This is neither a proven population p99 estimate nor
proof that the scanner causes the increase. A matched test module is now prepared
for both full native inputs, preserving both production prefixes. Standalone
builds with that identical module pass the same **seven tests** and list identical
test registrations. Full controlled native compilation/timing and larger tail
samples remain the next experiment; they have not been run or substituted for
these observations.

**Shipping readiness remains conditional.** The measurement module/registration
and SHA2 dev/lock edge were removed. The one-file shipping tree
`6fc5373ae8ff85eb3b613f44bbfff10be483f010` passes all ten merge-gate steps,
**2,318 passing test executions** reported by the single workspace test step
(unit/integration/doc tests), zero failures and six ignored, plus an ordinary
non-test server release build. This count is not an action-coverage denominator. All 1,631 tracked source hashes and the private staged tree
stay unchanged through validation. The production parser substring matches the
standalone and native candidate. This tree has no new commit/PR/merge; the wider
root edits are separate from the tested tree.

The audit adds one partial record for the existing `GET /api/meetings/stream`
identity: **202 records across 82 distinct identities**, HTTP **45 of 95 mapped,
50 unmapped**. A partial record proves only its bounded variants. The 82 identities span all
catalogue kinds; the HTTP denominator separately counts 95 explicit method/path
identities, of which 45 have at least one partial record. Neither is a completion
percentage. Existing records stay intact. The two earlier landings are SQLite
meeting-write reuse ([PR #2](https://github.com/bryantleft/bruhs-ai/pull/2)) and
capture request-body ownership ([PR #3](https://github.com/bryantleft/bruhs-ai/pull/3)).
The existing five-record qualification concerns unproven capture-proxy2 build-idle
checks caused by truncated macOS process names; it remains attached to those
records and does not discard their source/correctness evidence. Full argv0 build-worker checks, per-process profile
locks and identified socket cleanup remain checkpoints, not whole-host monitoring;
there is no operating-system tuning. Real capture/UI delivery, client
reconciliation, packet bytes, long-lived/concurrent/impaired connections,
configured remote/proxy/DNS/TLS paths and other application actions remain open.

Artifacts use `target/performance-sse-frames2-`: `comparison.json`,
`comparison.html`, `manifest.json`, the separate source/binary/build/run receipts,
`layout.json`, `main-comparison.json`, `repeat-comparison.json`,
`network-summary.json`, `ship-verified.json` and `matched-tests.json`.

## Capture event stream: matched TCP tails and parser-output probe

**The SSE scanner stays unmerged.** Matching the native tests and increasing each
case to 256 retained observations preserves a higher sampled p99 for the 16-event
fragmented stream in all four pairs. The isolated decoder remains faster. These
results make the network path part of the shipping decision; correctness and CPU
checks alone do not settle it.

The full interval starts when the prepared producer chunks are sent to the
fixture channel and ends after the final expected downstream frame is received
and framed. JSON/chunk preparation, equality checks, hashing and cleanup stay
outside. Both native variants use the frozen `2d1a29a6` main context, the same private
producer, production parser/pump/router and reqwest consumer across two IPv4 TCP
hops. Their normal test module is byte-identical, their complete server listings
contain the same 124 registered tests, and both report 527 passing focused core/server test executions,
zero failures and five additional ignored tests, plus formatting, all-target/all-feature
Clippy and release test-binary builds. Only `daemon.rs` differs across the 1,018
Rust/Cargo source paths in each epoch. Production prefixes match the forward scanner in shipping tree
`6fc5373ae8ff85eb3b613f44bbfff10be483f010` and main baseline exactly. The ordinary shipping source is unchanged.

Each native epoch has four version/case-order counterbalanced pairs, six cases
(1/16/128 events with 13-byte producer fragments or whole batches), **12,288
retained intervals**, 240 excluded warmups and 24 excluded smoke intervals plus
12 smoke warmups. No attempt is excluded. Quantiles remain per process. At
n=256, nearest-rank p99 is the third-largest observation and p95 the thirteenth;
these are sample statistics, not population-quantile or whole-action claims.

The uninstrumented 16-event fragmented p95 is higher in all four pairs too. Its
p99 results are:

| Pair | Main p99 ms | Scanner p99 ms | Difference |
| --- | ---: | ---: | ---: |
| A | 0.324125 | 0.515250 | +58.97% |
| B | 0.339583 | 0.384125 | +13.12% |
| C | 0.301917 | 0.408292 | +35.23% |
| D | 0.286041 | 0.359084 | +25.54% |

The whole-batch 16-event p99 is mixed: A/B increase 28.70%/26.71%, C/D decrease
2.22%/7.18%. Single-event results also vary. Large isolated maxima stay in the raw
data, including the fragmented 16-event C maximum of 0.315458 ms before and
4.179333 ms after. The earlier unequal-registration max32 experiment remains in
`target/performance-sse-frames2-network-summary.json` and its report section. Different registration and sample counts prevent assigning
the difference between those experiments to one cause.

A separate CPU diagnostic uses the six network-shaped payload/chunk configurations
at zero-based iteration 260, the last of the 261 warmup/retained
iterations; its input is reused within each decode-only row. Reconstructed expected downstream byte vectors across all 261
native iterations match each recorded native digest before the final input is
selected. The immutable uninstrumented exact-kernel timing binaries then produce
lower p50/p95/p99 in all six configurations and all four groups (12,288 retained
intervals; 240 warmups; smoke 24/12). The 16-event/13-byte decode median decreases
30.64–33.04%. Complete payloads/tails and retained output capacities agree. This
isolates decoder computation; it does not explain linked async/TCP behavior or
constitute a new allocation/RSS measurement.

The second native epoch adds a common `StreamExt::map` wrapper with one atomic
decrement per parsed payload and a timestamp on the final payload before pump
handling. Each batch records producer-send, final-parser-output and
final-downstream-frame timestamps. All cuts are ordered; delivery/reconciliation
checks still pass. This probe changes atomic/clock work and may change scheduling,
so its results remain separate from the uninstrumented epoch. Component quantiles
do not add to the quantile of total latency.

For the same fragmented 16-event case, the instrumented p99 differences are:

| Pair | Full interval | Send to final parser payload | Final parser payload to consumer |
| --- | ---: | ---: | ---: |
| A | +14.42% | +6.58% | +47.29% |
| B | -0.46% | +4.19% | +4.36% |
| C | -17.83% | -1.82% | -32.57% |
| D | +17.15% | +8.69% | +37.28% |

The probe does not uniquely locate the uninstrumented increase: its fragmented
16-event full p99 rises in A/D and falls in B/C, while each stage p99 rises in
three pairs and falls in C. The remaining parser-to-consumer median rises in all
four. The instrumented whole-batch 16-event p99 rises in all four (3.87–22.38%).
Neither the changed directions nor the component cuts identify an individual
operation as the cause. The production diff changes two mechanisms: buffer ownership/compaction and
forward scanning. Testing them separately can distinguish their contributions
without the extra probe. That is an ablation of the actual change, not a claim
that either mechanism is already established as the cause. The next comparison
will use the uninstrumented, matched-test TCP protocol.

Both epochs retain complete equality for every 1/16-event frame. The 128-event
cases retain ordered delivery and explicit reconciliation for gaps; their times
remain diagnostics. In the uninstrumented epoch, each version receives 296,960
input events, delivers 231,424, skips 65,536 and emits 1,024 reconciliation notices
across 6,144 timed batches. Instrumented totals are retained separately in
`target/performance-sse-frames4-main-comparison.json`.

Build-worker checks use full argv0, four shared profile locks are held per process,
and previously identified fixture socket pairs clear before the next process.
These pre/post observations do not establish continuous host quiescence. Producer
chunk sizes and payload counts do not measure packet boundaries or wire bytes.
No production timeout, queue bound, retry, pooling, proxy or TCP setting changes.

The previously gated one-file shipping tree
`6fc5373ae8ff85eb3b613f44bbfff10be483f010` still has its full ten-step gate,
2,318 passing executions, six ignored and ordinary non-test server release proof.
Those checks were not rerun because that shipping source did not change. No new
commit, PR or merge is created. The wider dirty root is separate from this frozen
source context. All previous 202 evidence records, two landings and the five-record
process-readiness qualification stay intact; this adds one partial record, making
203 records across 82 mixed-kind identities. HTTP mapping remains 45 of 95 explicit
method/path identities, with 50 unmapped. All 45 mapped HTTP identities are
currently `evidence_source_changed` against the dirty root; this record differs
from root in 220 recorded source paths. No current-root or whole-action
acceptance follows.

The 202 historical records are preserved in
`target/performance-sse-frames3-ledger-before-publication.json`. The current
source/action inventory and the new publication receipt pin the counts above.
The principal receipts are:

| Claim | Artifact |
| --- | --- |
| Uninstrumented native timings, tests and source capsules | `target/performance-sse-frames3-main-comparison.json`, `target/performance-sse-frames3-main-before-manifest.json`, `target/performance-sse-frames3-main-after-manifest.json` |
| Native-derived CPU inputs and timings | `target/performance-sse-frames3-cpu-manifest.json`, `target/performance-sse-frames3-cpu-comparison.json` |
| Instrumented native timings and cuts | `target/performance-sse-frames4-main-comparison.json`, `target/performance-sse-frames4-stages.json` |
| Unchanged shipping-source gate | `target/performance-sse-frames2-gate-source.json`, `target/performance-sse-frames2-ship-verified.json` |
| Mapping counts and publication | `target/performance-action-inventory.json`, `target/performance-sse-frames3-publication.json` |
| Interactive uninstrumented comparison | `target/performance-sse-frames3-comparison.html` |
Real capture/UI delivery, client reconciliation, long-lived/concurrent/impaired
streams, cancellation/retry variants, packet-level work and configured
remote/proxy/DNS/TLS paths remain open.

## Capture event stream: four-way parser comparison

The stream candidate remains unmerged. Separating compaction from scanning shows
that both forward-scanning variants retain higher sampled p99 for the 16-event
fragmented TCP case in all four groups. Compaction alone has mixed native tails.
This narrows the next investigation to the scan path and its interaction with the
linked runtime/compiler; it does not establish a unique cause or population p99.

The four variants are baseline, per-chunk compaction/borrowed frames only,
forward scanning only, and both changes. Three kernels match the previous exact
source archives; scan-only replaces the baseline scan and removes its overlap
rescan. All four pass the same seven framing/payload tests, including the binary
oracle. Their native test modules are byte-identical and list the same 124 server
tests. Each passes 527 focused core/server executions, zero failures and five
additional ignored tests, formatting, all-target/all-feature Clippy and release
builds. Only `daemon.rs` differs across their 1,018 Rust/Cargo source paths.

Four Williams-design groups put each variant in each position once and use each
directed adjacent pair once. Every variant receives two forward and two reverse
case orders. The private native fixture is the original uninstrumented producer,
production daemon/pump/router and reqwest consumer across two IPv4 TCP hops.
Its clock covers prepared producer send through receipt/framing of the final
expected downstream frame; preparation, checks and cleanup stay outside.

The native experiment retains **24,576 intervals**, 480 warmups and smoke 48/24;
no attempt is excluded. Each case/process has 256 retained samples, so p99 is the
third-largest observation. Quantiles stay per process. For the 16-event
fragmented case, p99 differences against baseline are:

| Variant | A | B | C | D |
| --- | ---: | ---: | ---: | ---: |
| Compaction only | -2.15% | -3.47% | -0.79% | +16.99% |
| Forward scan only | +5.79% | +8.55% | +8.78% | +9.59% |
| Combined | +74.16% | +4.62% | +3.71% | +2.48% |

The scan-only 16-event whole-batch p99 is also higher in all four, including a
+113.46% C observation. Combined whole-batch tails are mixed. Single-event and
other component comparisons remain in the raw results. Source, code generation,
scheduling and host variation remain possible contributors; order balancing does
not prove causality or continuous host quiescence.

The separate uninstrumented CPU matrix has 40 configured rows: 34 prior edge/stress
rows and six native-derived inputs. It retains 20,480 intervals and 3,200 warmups,
with smoke 320/160. Combined p50 is lower in all four groups for all 40 rows;
scan-only does so for 37, compaction-only for 29. Compaction alone has six rows
with consistently higher medians, all one-byte fragment configurations.
For coalesced 8,192-event multiline input, compaction removes the repeated-shift
cost (about 97.4–97.5% lower median); scanning alone leaves it mostly unchanged.
For one-byte fragmented input, forward scanning supplies the large CPU saving.
At 32 CPU samples, p99 is the maximum; mixed tails remain retained.

There are 960 separate serial allocation probes. All six observations per
case/version agree. Baseline/scan-only allocation statistics match in all 40 rows,
as do compaction-only/combined. Coalesced multiline input drops from 25,061 to
16,386 allocation calls under compaction, with unchanged retained output capacity.
These are allocator requests and logical live bytes, not RSS or async-future size.

Every native 1/16-event frame passes full equality. Each variant has 6,144 timed
batches, 296,960 input events, 231,424 delivered events, 65,536 skipped events and
1,024 reconciliation notices. The 128-event times remain delivery diagnostics.
Readiness uses full argv0 checks, per-process shared profile locks and identified
socket cleanup. No production timeout, queue, retry or TCP setting changes.

No production source change, commit, PR or merge occurs in this pass. The prior
one-file tree `6fc5373ae8ff85eb3b613f44bbfff10be483f010` retains its existing full
gate proof; it was not changed or retested. The owned checkout is archived and
removed, preserving all 1,902 original working-path contents, index and stashes
before explicit report/log publication. The new ledger record remains partial.

Receipts use `target/performance-sse-frames5-`: `variants.json`, `design.json`,
`comparison.json`, `main-comparison.json`, source/build/run receipts and
`cleanup.json`. The normalized `action-observations.json` contains 736 bounded
observations with exact raw pointers: 640 component, 64 complete-delivery network
variants and 32 delivery diagnostics.

## T3 screenshot targets and the working action log

**No exact faster-than-T3 result is verified for the seven supplied workloads.**
The official source PRs confirm the screenshot's figures and clarify that they
mix store reads, formatter CPU, bootstrap bytes and impaired-link completion.
Current implementation similarities do not establish equivalent timing wins.

| Target from the screenshot | Current finding |
| --- | --- |
| Open 41,000 items, 31 ms | T3 measures the first server snapshot read. Native chat has viewport rendering and web has keyed rows; web storage still decodes a whole per-thread message record. No matching 41,000-item read/open benchmark. |
| Earlier history, 0.7 s | T3 measures a cold older-page read in a 45,154-item/315 MB fixture. Current web storage's whole-vector message record does not provide equivalent bounded history paging. |
| Send at 75,000 items, 90 ms | T3 measures a warm history read, not a live send. Bruhs sends a prompt plus thread/provider session IDs instead of full history, but client/orchestration costs at this size remain unmeasured. |
| Mobile launch, 0.7 MB | T3 counts bootstrap socket data across 11 environments. No equivalent multi-environment mobile workload is verified for the inspected single-API-base web client. |
| Mobile reconnect, 2.2 s | T3 measures a warm reconnect beyond the replay cap at 1.6 Mbit/s and 300 ms. Existing loopback/responsive checks do not measure this cached snapshot/resume workload. |
| Terminal attach, 751 ms | Bruhs already caps replay at 512 KiB. Matching slow-link, retained-state, Unicode/ANSI, compression and client-apply timing are still required. |
| Reasoning preview, 1.3 ms | T3's figure excludes parsing/layout/paint. Bruhs's collapsed rows use a fixed label and defer the body, which avoids that formatter but supplies a different preview. No equivalent display/timing win follows. |

The source PRs are [thread open #34490](https://github.com/pingdotgg/t3code/pull/34490),
[older pages #34573](https://github.com/pingdotgg/t3code/pull/34573),
[send history #35130](https://github.com/pingdotgg/t3code/pull/35130),
[mobile bootstrap #35072](https://github.com/pingdotgg/t3code/pull/35072),
[reconnect #32287](https://github.com/pingdotgg/t3code/pull/32287),
[terminal #33942](https://github.com/pingdotgg/t3code/pull/33942) and
[reasoning #30038](https://github.com/pingdotgg/t3code/pull/30038).
Later reasoning-source bounds also landed in
[#23001](https://github.com/pingdotgg/t3code/pull/23001), so current code must be
pinned when benchmarking. The screenshot, inspected source hashes, merged PR
commits and exact workload boundaries are recorded under
`target/performance-t3-gaps1-{matrix,upstream}.json`.

The user also requires a final log of every supported action and how fast it is.
The living [action log](performance-action-log.md) and JSON companion now list 254
source-discovered entrypoints and the 736 normalized bounded observations above.
Another 1,049 binding search references still need semantic review. These counts
are not an exhaustive user-action denominator. Earlier partial timings remain
linked while their numeric distributions are normalized; unknown values stay
explicit. The goal remains active until the semantic inventory, timings, network
conditions, source/build/hardware context and remaining variants are verified.

The ledger has 204 partial records across 82 mixed-kind identities; HTTP mapping
remains 45 of 95, with 50 unmapped and current-root source differences still
qualified. All 203 historical records, two landings and five correction
annotations remain intact. The T3 checks and complete action log are explicit
completion requirements, not grounds for marking the current audit complete.

## Next loop

The seven user-supplied T3 workloads now have explicit source-grounded
boundaries and gap findings. No exact faster-than-T3 result is verified. Prioritize
matching the41k/45k/75k history workloads, supported mobile/network semantics,
slow-link terminal completion and equivalent reasoning previews. Maintain
`docs/performance-action-log.md` and its JSON companion; semantic inventory,
historical timing normalization and complete action/context coverage are required
before the overall goal can be complete. Other action-audit scope below remains.


1. Continue full web transcript initial mounting, physical browser paint,
   code postprocessing, and the remaining long native reply shapes. Native
   closed-code tail updates and ordinary Mermaid mentions now avoid repeated
   parsing/geometry work. Measure open code fences, single very long paragraphs,
   container/definition transitions, initial mounting and the remaining diagram
   AST parse; the guarded tail path can speculatively parse a tail before a full
   fallback. Large native prose still has a roughly 14 ms foreground floor in
   the current fixture. Emoji-heavy chat
   opening now avoids the slow fallback path without deferring selection work.
   Offscreen containment was rejected after whole-history selection regressed.
   The transcript's scoped primary-font features now reduce large-history
   layout without changing fallback script shaping; complete 10,000-row DOM
   construction still costs roughly 60 ms before layout. The live-capture
   renderer is a separate surface to measure. Web transcript updates retain stable
   rows/editors; chat uses indexed row lookup and now retains prose DOM through
   text updates, completion and same-thread metadata edits. Initial full DOM
   size and complete Markdown parsing remain costs. Profile code-block DOM
   processing/tails before choosing further fast paths; its primary tail is
   mixed despite a lower median, and repeats remain recorded.
   Cold native SVG work is now off the UI thread with cancellation and streaming
   coverage; the unfinished-math scanner's repeated suffix scans are also fixed.
   Bound work without making records
   unreachable or disrupting selection, focus, or native/browser find.
2. Measure record construction/import throughput, shared-record memory use,
   dense People rendering and other large owned collections, plus cold storage,
   contention, debounce/receipt latency and recovery. The continuous native cache
   component now has a shared-decoder original/current comparison: fragment-only
   loads improve; mixed replacement is roughly flat. Whole workspace
   loading now also has a controlled warm file-read-through-Root fixture across
   seven shapes and primary/recovery routes: record transfer helps the large
   knowledge/record/mixed cases, while small controls are mixed. Owned saved-thread
   restoration now has separate copy-stress and alternating-conversation evidence.
   Initial rendering and repeated native sidebar search/redraw now have paired
   evidence through 2,048 threads; a lazy flex minimum fallback removes discarded
   measurements. Saved-chat construction also skips an immediately replaced cwd
   lookup; the earlier startup tradeoff is variable on repeat, with all cohorts retained.
   Full-workspace fixtures now flush prior entity cleanup outside timing; earlier
   total/settle samples include that prior cleanup and need remeasurement before
   broader startup acceptance. Those test fixtures use dummy text metrics and no
   GPU renderer. The current sidebar renders visible rows and has matched normal
   native font/frame evidence through 2,048 pinned general threads. Measure cold
   first use, mixed/project/shelf workloads, scrolling, physical input/presentation
   and the other remaining native actions next. Controlled native auth/restoration
   comparisons now cover actual file/Workspace loading, all initial status delivery
   and the second following native callback after an intervening draw opportunity
   through 128 pinned general chats. The window, platform, runtime and fonts are
   initialized first; an existing interpreter, temporary script and counter I/O
   replace real account queries. The active chat can be usable before all metadata
   settles. Measure first-interactive/active-chat-first readiness, total/cold launch
   and mixed workspaces before broader startup acceptance. Revalidate affected
   historical entries against the current source before broader acceptance.
   Separate samples show JSON parse/unescape in the actual reader, plus
   sidebar/text-shaping/layout paths. Saved-file SIMD UTF-8 validation now has
   complete paired loading evidence across 36 cases; small controls and some tails
   remain mixed. Recovery now skips its unused backup digest, while primary raw-byte
   versions and conflict protection remain intact. Large backup cases improve in both
   orders; primary controls and some tails remain mixed. Measure large early-invalid
   JSON before broad error-path acceptance. Continue JSON read/decode/hash, initial rendering
   with many threads, small-workspace fixed costs, OS-cold storage, contention,
   allocation/memory, more distributions and physical startup/paint.
   Foreground people/finance
   snapshots now share records, with both run orders measured through 5,000
   people and 100,000 finance records. Warm acknowledged save completion has complete-byte/backup
   fixtures through 5,000 people and 100,000 finance records; hardware SHA-256
   and early obsolete-request rejection reduce worker CPU. Filesystem tails
   remain variable. Meeting-heavy,
   knowledge-heavy and mixed snapshot/encoding fixtures also remain available;
   meeting text already shares ownership. Preserve the acknowledged
   save/recovery protocol.
3. Measure cold WASM startup and input-to-next-paint in the browser, plus native
   physical input/GPU presentation latency.
   Warm sidebar comparisons now include normal native fonts, layout and frame
   construction, while earlier headless fixtures remain separate. Both exclude
   physical input and GPU completion. Compute and forced-layout fixtures cannot
   establish INP, LCP, frame rate, or a user-visible all-actions latency claim.

4. Extend terminal lifecycle coverage to successful concurrent launches and
   cancel during native
   paste/checkpoint preparation, along with slow filesystem and file-index
   contention fixtures.
5. Measure network setup/reuse, DNS/TLS, provider/proxy hops, wire sizes,
   streaming/backpressure, timeouts/retries/cancellation and concurrent or impaired
   links for every action. Generation TCP now has a completed counterbalanced
   repeat using fixture socket readiness. Fresh-socket burst resilience, higher
   controls, allocations and independent variability remain open. Native local-
   context now has actual caller/router/private-store evidence and a local shared
   dedicated-client cache. A frozen three-variant comparison does not support replacing
   asynchronous initialization with a synchronous cache: first-use controls remain
   often higher and no broad acceptance follows. Keep the existing local cache;
   inspect client/pool lifetime, scheduling and retention before changing policy.
   Preserve proxy/redirect/loopback restrictions and retryable initialization.
   The isolated experiment has five configuration/fixture differences from root;
   full native/root validation, allocations and independent variability remain open.
   Capture forwarding's payload-copy removal now has a fresh two-round repeat:
   50 listening-daemon latency cases,12,000intervals and full52-case correctness
   coverage including the unchanged unavailable deadline. Four affected pooled
   medians are lower in all four pairs; fresh results/controls/tails remain mixed.
   The old contested reverse timings remain excluded. Build locks are released
   between measurement processes; no new attempt needed exclusion. Source shows
   less copying, without uniform latency or numerical acceptance. The isolated
   two-file clean-main tree passed the full gate (2317Rust/62site tests and
   typecheck); PR#3 is merged into main at2d1a29a6 with the same tested tree.
   Only the two forwarding files shipped. Preserve concurrent desktop
   edits and captured-source drift. Real capture, broader timeout/refusal/stall,
   independent workload/host variation and remaining network layers stay open.
   Revalidate affected paths against current sources: later concurrent core edits
   appeared after the saved-binary comparison and are preserved.
   Measure provider time-to-first-event, capture throughput, and Google sync with
   representative fixtures before any concurrency or cache-policy changes.
   Warm 64 MiB cached-model verification is now measured with a complete-hash
   corruption check; cold model loading and the audio/inference pipeline remain
   separate measurements.
   Native finance transaction reads now have improved ASCII handler scans,
   real Unix-socket timings and full stdio/native fixture evidence. Tool discovery
   avoids repeated catalog copies, and standalone empty-state reads are timed.
   Generic Tasks/Calendar/Capture/Knowledge/Inbox scans now reuse canonical JSON
   storage, with actual handler and full-stdio measurements across all five.
   Sleep/Recovery use the existing one-field matcher for their public ID. Two
   isolated record-scan studies support that caller choice. The combined native
   comparison against the original baseline has lower p50 in all five measured
   nonempty cases per section, but does not isolate the two edits. Favorable and
   unfavorable median patterns both require causal caution. Unfiltered pages,
   other controls and tails remain unresolved. The separate same-binary A/A
   experiment establishes no numerical tolerance or equivalence. Investigate
   read controls using the completed scratch3/current-build comparison before
   broad acceptance or merge. Direct facade dispatch removal is source-verified;
   the paired read observations remain mixed and higher controls stay open.
   The HTTP catalogue includes 95 explicit method/path entries; 39 have partial
   mapped actual HTTP variants. Task-stream encoding has separate constructor,
   allocation and paired HTTP evidence, with finite controls still unresolved.
   Persistent people/meeting reads now have memory/private SQLite baselines;
   filesystem reads use confined temporary directories and an unborn repository.
   A single-worker directory candidate improves the large case, while no-visible-
   subdirectory controls, concurrency, cancellation and memory remain open.
   File-child browsing now retains at most the existing 500 best candidates,
   with stable-tie/cutoff tests, paired HTTP and owned-selection CPU/memory evidence.
   Large-input requested peaks fall sharply; the 501-entry transition and reversed
   input can cost more CPU/memory, while some HTTP tails/controls are higher.
   Measure remaining directory-batch handoffs, cancellation/error behavior and
   dense-name/concurrent HTTP memory before acceptance.
   Contact writes now add six partial HTTP mappings across private memory/SQLite
   fixtures with full replies, versions, events, state and reopen checks. Reusing
   the SQLite name-insert statement improves alias-heavy writes; ordinary cases
   and unchanged memory controls remain mixed. Quantify process-to-process
   variation before broad acceptance, and measure unchanged-name index rewrites.
   Enrollment covers missing-contact rejection only; success still needs capture.
   Meeting/local-focus routes add eleven partial HTTP mappings across 372 variants.
   SQLite saves now validate distinct referenced contacts once and lazily reuse
   the relationship insert statement. Dense repeated/unique reference cases
   improve; ordinary cases and unchanged controls still need follow-up. UUID and
   real-clock witnesses precede narrow canonicalization; full state/events,
   relationship retargeting, rollback and reopen checks pass. The shared save
   helper and regression test landed in PR #2; measurement fixtures and the
   broader audit remain local. Measure allocations, concurrent/cold storage,
   live streams and physical UI delivery next.
   Generation receipts add four partial HTTP mappings with real TCP socket reuse
   and fresh-connection witnesses. Known-receipt SQLite fencing avoids one repeated
   read/decode. The counterbalanced repeat supports the large successful-receipt
   case; pending fencing and higher controls prevent broad speed acceptance. The
   fix remains local pending further review and a full merge gate.
   The SSE forward scanner remains local. The four-way native comparison
   associates higher16-event fragmented p99 with both forward-scan variants;
   compaction-only has mixed native tails. Combined CPU medians improve in all
   40configured rows, and compaction removes the coalesced shifting/allocation
   cost. Investigate the scan path/linkage under a changed hypothesis; preserve
   all prior native epochs and the overflow/reconciliation contract. No source
   change or merge occurred. Capture-proxy2 readiness remains qualified by its
   correction; use full-argv0 checks.
   Broaden the remaining 50 HTTP identities and storage error/corruption, merge,
   reopen, cold/concurrent and repository variants. The handler plan is a historical design snapshot.
   The three workflow tools now have partial native measurements and a
   validated pending-review scan fix. Many previews, long jobs/steps/records,
   save completion and other rollback domains remain. Broaden HTTP/palette and
   normal UI coverage before another isolated query-tuning loop. Preserve std
   semantics, offsets, projections and permissions. Continue full
   Unicode casing, JSON serialization, concurrency, normal native/browser actions
   and remaining workflow, palette, HTTP and semantic coverage.
   Measure cheap isolated snapshots and background/indexed reads, plus large
   escaped/error-path memory and latency. Projected discovery pages now have
   paired evidence across Finance/Health/Meeting/Note/People/Chat variants. Seven
   exact-ID context/history variants now have paired handler/full-stdio evidence.
   Warm unchanged transcript maps reuse complete validation with mutable-borrow
   invalidation; cold/post-mutation validation, alternating meetings, larger records,
   byte/error timing and other distributions remain. Standalone previews
   and complete audits now have paired evidence through 10,000 prior workflows; larger
   histories, long operations, output CPU/transport and contention remain. Reusing frame
   and consumed-text storage now avoids repeated retained-memory growth in the measured
   10,000-workflow fixture; cold first allocation, larger frames and other allocators need
   separate checks. Measure cold
   hydration/connection enablement, concurrent load, cancellation and error tails.
   The current standalone catalog cannot populate thread/task maps.
6. Extend native calendar measurements to OS input/GPU frames during
   drag/resize and to browser layout/paint. Repeated native merge/projection
   work and quadratic dense lane scans are now fixed with optimized
   computation/test-layout diagnostics and behavioral regressions. Calendar
   provider sync remains a separate network/cursor/delivery measurement.
7. Expand the reproducible source action catalogue and its palette dispatch
   links and reviewed partial-evidence ledger into semantic action/variant coverage and
   map each entry to its actual computation, UI, transport and acknowledged
   persistence evidence. Audit remaining forms, context menus, shortcuts,
   recovery states and MCP modes. The catalogue is a discovery aid, not proof
   that all actions have been measured.
   Empty non-project new-thread command and dispatched-action paths now avoid a
   duplicate initial Claude auth query, with complete created-state and later
   provider-switch checks. Measure interactive palette/input and normal native
   latency, project/role variants and acknowledged persistence separately.
   Thread-catalogue publishing now reuses owned storage, with paired isolated
   method and capacity diagnostics. Measure normal native frame/action impact,
   other mutation positions and long-lived mixed-workspace memory; the value
   comparison remains linear in thread count and title bytes.

The native command-palette meeting-note fixture also ran from the baseline
release binary: 10,000 indexed notes, a complete `notequartz` input, warm
input-plus-layout p50/p95 **35.922 / 37.195 ms**. It returns the existing bounded
record result count. This is a whole-input diagnostic, not a measured single
keystroke INP value or evidence that 10,000 result rows are mounted. It is not
used to claim a note-search speedup; the sixth-pass fixture separates the input
and selection actions and measures the thread-projection change independently.

## Terminal replay over a slow connection

The terminal already bounded replay to 512 KiB, but transferred that history without compression. The new candidate compresses remote web replay snapshots, keeps loopback connections raw, and leaves live output uncompressed. This improves the measured Bruhs slow-link path. **It does not establish a matched performance win over T3.** The overall performance goal remains active.

| Workload | Slow-link median before | After | WebSocket payload before | After, including acknowledgement |
| --- | ---: | ---: | ---: | ---: |
| High entropy | 973.60 ms | 765.50 ms | 524288 bytes | 392001 bytes |
| Repetitive logs | 971.45 ms | 150.10 ms | 524288 bytes | 11672 bytes |

Each case retains six observations after one warmup. The paced TCP stream supplies 5 Mbps and 120 ms round trip. Timing starts before production terminal creation/WebSocket attach and ends at the final xterm write callback. Assets are warm and the timed parser is unmounted at 1000 columns by 24 rows. Paint, remote DNS/TLS, packet loss and a real mobile device are excluded. The proxy delays/paces application bytes and does not model remote TCP connection setup. Three calibration transfers of 524288 bytes took 963.27–967.11 ms. Message payload counts exclude HTTP/WebSocket/TCP/TLS overhead; separate proxy logs retain application-stream counts.

An optimized private fixture uses the production router/PTY manager and two Tokio workers; it does not start the full application. Both synthetic histories have 4900 seeded lines (4652897 high-entropy bytes and 4885465 repetitive bytes). The real PTY manager's exact last 524288 bytes are checked before timing. Every accepted run preserves those replay bytes and the captured xterm state: retained cell text, width, colors and selected styles, wrapped lines, cursor and buffer positions, and dimensions. This establishes parity with the bounded baseline, not reconstruction of the full original history or every terminal property.

Local testing caught a cost in the first prototype: entropy replay rose from 9.65 to 12.65 ms with compression. The final client therefore keeps loopback connections raw. Final direct medians were 9.45 to 9.70 ms for entropy and 9.65 to 9.40 ms for repetitive logs. These small samples report observed medians; they do not prove statistical equivalence or population tail latency. The impaired-link fixture maps a reserved remote hostname to its private loopback proxy so the real client's remote-selection path executes without external DNS.

The first prototype also failed new-client/old-server compatibility because the browser rejected an unacknowledged WebSocket subprotocol. The [browser handshake standard](https://websockets.spec.whatwg.org/#opening-handshake) requires that rejection. The final request uses a query parameter and an explicit text acknowledgement before interpreting framed binary output. Older servers ignore the query and remain raw. Compression runs in the blocking pool only for snapshots; live output stays uncompressed. Component selection measurements remain in the linked raw receipts.

Browser validation passed all 56 final timing runs, three old/new client-server or missing-decompressor combinations, a mounted terminal with live Unicode input, and six adversarial subcases. The mounted test preserved the exact snapshot followed by one green `LIVE_雪_😀` line. Malformed gzip, unknown encodings and oversized expansion failed visibly. Pending decode disposal left replacement sessions untouched. Three decoder console errors were expected by those negative tests; no unexpected harness errors or unhandled rejections were recorded. Two final diagnostic snapshot calls timed out after assertions and restoration completed; prior visual captures, assertions and tab-close receipts remain available.

An early measurement wrapper attributed a previous socket close to the next attach; that series is excluded. A metadata fetch failed before a compatibility attach and was replaced with metadata transfer between owned tabs. One final warmup completed before its tool-side validator failed; its original browser result was recovered without repeating the execution. These diagnostics and the rejected prototype remain in the raw artifacts. The first gate run passed nine steps but Trunk rejected inherited `NO_COLOR=1`; the corrected full gate passes all ten steps (2320 Rust tests passed, zero failed, six ignored), and the ordinary release server build passes. Only four production files form the shipping candidate. Synthetic fixture methods, routes, feature changes and assets are excluded.

T3's screenshot reports 750.5 ms on a Windows ConPTY fixture with different history and a client-reducer completion boundary. The Bruhs data, host and client work differ, so the seven-target comparison still records zero verified wins. Real remote attach/reconnect/resync, broader terminal semantics and every other action still require their scoped evidence.

The living action log gains 18 bounded terminal observations, including superseded prototypes; eight final observations appear in its terminal table. It remains explicitly incomplete. Semantic inventory review and historical numerical normalization are still required before the final log can claim every supported action.

Receipts: [final slow study](target/performance-terminal1-browser-paired-v2.json), [local study](target/performance-terminal1-browser-local-v2.json), [compatibility](target/performance-terminal1-browser-compatibility-v2.json), [mounted/live](target/performance-terminal1-browser-live-v2.json), [adversarial](target/performance-terminal1-browser-adversarial-v2.json), [normalized timings](target/performance-terminal1-action-observations.json), [source and binaries](target/performance-terminal1-paired-v2-input.json), [shipping input](target/performance-terminal1-shipping-input.json).

### Terminal replay landing

The terminal replay change landed in [PR #4](https://github.com/bryantleft/bruhs-ai/pull/4), main commit `7e060f60ff41b378e05763230b2bf91f37a27e09`. Its merge tree exactly matches the isolated tree that passed all ten gate checks and the ordinary release server build. Only four files shipped; fixture helpers and other workspace changes remain outside this merge.

The root workspace was brought forward and verified against a snapshot taken immediately before integration. The expected four files changed; the other 1901 working files and existing stashes were preserved. These landing checks do not promote the synthetic timing measurements to full-action or T3 acceptance. The broader goal remains active, and the action timing log remains incomplete.

## Web conversation storage baseline

The large-history storage gap remains. The current web client reads and decodes a whole per-thread record, retains a raw original for conflicts, and rewrites that record for an append. An optimized WASM fixture measured the real IndexedDB load/recovery and conflict-checked commit paths without starting the main application or any provider/API work.

| Synthetic history | Messages / ordered parts | Input JSON bytes | Load + recovery median | Local append commit median |
| --- | ---: | ---: | ---: | ---: |
| Small control | 100 / 100 | 42465 | 0.60 ms | 1.20 ms |
| Mixed history | 460 / 41000 | 13799909 | 168.50 ms | 330.20 ms |
| Many messages | 45000 / 45000 | 19471589 | 170.00 ms | 352.50 ms |
| Many messages | 75000 / 75000 | 32474804 | 303.40 ms | 612.50 ms |
| Large tool output | 100 / 650 | 16622135 | 89.70 ms | 112.70 ms |

Each median has three retained observations after one warmup. Load starts before the actual read-only loader and ends after raw-original retention and display recovery. Append starts immediately before the production conflict-checked save and ends after its terminal commit event. Preparing the expected/draft records, verification and restoration are outside the append timer. Full UI activation, DOM/paint, remote network, physical cold I/O, provider work and server send are excluded. These local commit times cannot be compared with T3's 90 ms server-history boundary. Input byte counts describe the JSON representation, not physical IndexedDB bytes or network traffic. Browser version was not captured, so this remains a preliminary baseline.

These preliminary probes do not support adopting blanket batching. The existing metadata batch helper reduced mixed-history decode from 125.4 to 90.1 ms and encode from 107.0 to 60.6 ms. On 45000/75000-message inputs its decode medians were higher than the current decoder. For large tool output its encode median rose from 4.9 to 35.4 ms. These are component observations on warm input values, not a changed production path. No generic codec change was adopted.

The fixture completed 126 runs: six smoke calls, thirty warmups and ninety retained calls across five cases and six operations. Counts, canonical hashes, display hashes and append readback/restoration checks passed. Only the generated appended message UUID was normalized for comparisons; the typed writer checked the actual UUID before restoration. Recovery preparation was small relative to loading/commit in these cases. Some control-stage readings were zero at the browser clock resolution, which does not mean zero work. A rejected untimed preflight inspection is retained separately; no timed run was rejected or repeated. Browser diagnostics recorded no errors or denied requests.

This uses main `7e060f60` plus a private feature/export/clock/visibility overlay. The actual load/save bodies are unchanged. All data live on a private incognito origin with the production version-11 store layout. Cleanup deleted the five synthetic histories, confirmed all eighteen stores empty, deleted the private database and closed the tabs. The owned HTTP fixture was stopped. No application change or new PR was shipped in this pass.

The action timing log gains forty bounded observations: twenty codec probes, ten load/recovery stages and ten load-or-commit rows. None establishes complete action or T3 acceptance. The next phase must trace every full-history consumer, then validate a replacement that reduces history work per operation while preserving originals, conflict groups and recovery; the [storage work note](target/performance-history1-next-work.md) records those requirements.

Receipts: [raw browser runs](target/performance-history1-browser-probe.json), [normalized observations](target/performance-history1-action-observations.json), [source/build/fixture hashes](target/performance-history1-baseline-input.json), [cleanup](target/performance-history1-browser-cleanup.json).

## Web conversation header and content codec

The hybrid candidate improves converted-history local reads and appends in the measured synthetic workloads without changing the complete-history API. It stores small message headers in JSON and keeps content strings and arbitrary tool inputs in structured-clone arrays. The isolated shipping tree passed all 10 merge-gate steps: 2,328 tests passed, zero failed, six existing ignores. [PR #5](https://github.com/bryantleft/bruhs-ai/pull/5) merged at `0c140e8db1fef716c4ebb10276d91cbe6c322ae5`; its tree `2dce15b6cdc32dbbb8c787fb8c32a8094597c9dd` equals the gated tree. Root main was fast-forwarded while preserving existing work. The broader action and network audit remains active.

The 75,000-message synthetic fixture improved from 282.1 to 115.3 ms for read plus recovery preparation, and from 557.0 to 199.2 ms for a local append commit after conversion. Its first append from legacy storage improved from 571.35 to 346.35 ms. Reading an untouched legacy record remains on the existing path. These are local storage boundaries, not whole UI or T3 acceptance.

| Synthetic workload | Legacy first read ms, before / candidate | First legacy append ms, before / candidate | Converted read + prepare ms, before / candidate | Converted local append ms, before / candidate |
| --- | ---: | ---: | ---: | ---: |
| small-control | 0.50 / 0.60 | 1.10 / 0.80 | 0.50 / 0.25 | 0.95 / 0.55 |
| mixed-41k-parts | 126.55 / 123.15 | 281.10 / 177.70 | 127.15 / 55.15 | 275.30 / 111.40 |
| messages-45k | 158.00 / 152.30 | 331.60 / 200.15 | 162.45 / 65.15 | 333.70 / 111.70 |
| messages-75k | 264.50 / 261.80 | 571.35 / 346.35 | 282.10 / 115.30 | 557.00 / 199.20 |
| large-tool-output | 29.70 / 28.85 | 51.30 / 42.35 | 28.35 / 15.60 | 51.90 / 27.95 |
| long-reasoning | 5.10 / 5.10 | 12.10 / 8.50 | 5.05 / 2.55 | 11.60 / 5.95 |
| escaped-output | 11.45 / 11.80 | 27.85 / 20.15 | 11.30 / 5.40 | 26.10 / 12.45 |
| structured-fallback | 0.60 / 0.50 | 1.10 / 1.10 | 0.50 / 0.50 | 1.05 / 1.05 |

Table values are medians of six retained samples after one warmup. The 448 scheduled intervals include 64 warmups and 384 retained intervals; implementations and forward/reverse case order were counterbalanced. Separate setup receipts record 112 untimed typed rewrites or baseline append/restores. Small differences in the control, fallback and legacy reads are within the resolution/variation of this run, not a universal regression or speedup claim. Sample p95/p99 are the maxima of six observations and do not establish population tails.

The mixed-41k-parts fixture is 460 messages with 41,000 ordered parts. The 45k and 75k fixtures contain that many messages and parts. Large tool output, long reasoning and escaped output are separate stress cases. The structured-fallback fixture uses supported positive-infinite start timestamps: exact float bits are checked before seeding, and request telemetry confirms structured storage after writes. Input JSON byte counts are representation sizes, not disk or network bytes. Canonical JSON hashes alone cannot distinguish non-finite values; the fallback bit checks and typed append readback cover them.

Chrome 157.0.8103.52 ran in private headless contexts on the macOS arm64 host. The main application was unmounted, with provider/API/event-stream/socket access disabled. Read-plus-prepare ends after actual read-only loading, typed-original retention and display recovery. Append starts after preparing the expected/draft records and ends after the terminal IndexedDB commit. Canonical hashing, readback and typed restoration are outside its timer. Wrapper and request-event timings remain diagnostic. Other user/system activity was present; a single process snapshot is not continuous quiescence. No physical cold-I/O, UI/paint, mobile, remote-network or server-send acceptance is claimed.

Compatibility and failed candidates:

- Web full-history consumers traced for this change include activation/retry, offscreen queued starts, provider context/retry, streaming snapshots, rewind/edit, import, copy/export, cross-thread reads, find/selection/scroll and diff context. The API still materializes complete histories. Bounded reads and incremental writes remain separate work.
- The JSON-only full-envelope prototype completed 280 intervals (240 retained), but large-tool-output converted reads regressed from 28.6 to 39.25 ms and local appends from 51.1 to 77.25 ms. It was rejected.
- The compact JSON-only prototype passed 44 ordinary browser checks after fixing an explicit-key validation gap, then rejected valid 140-level tool input that the structured codec roundtrips. It was rejected before retained timing. Serde-wasm-bindgen ignores undeclared struct fields, so the final codec explicitly checks the envelope keys.
- The hybrid candidate passed 62 browser checks, including persisted deep tool input, missing/extra content arrays, malformed/unknown encodings, exact floats/fallback, readonly recovery, stale grouped saves, abort, quota, parent/cascade, workflow rollback and schema upgrade. The native codec suite has eight tests. A redundant clone in one native test was removed after the first isolated clippy failure; runtime code is unchanged.
- IndexedDB version 12 gates the new format while retaining all 18 stores. Reads preserve originals. Only a successful original-checked save converts a legacy record; special header numbers use the existing structured codec. The upgrade closes old connections before this format can be written; version-11 clients refresh to load the compatible codec. All writer, cascade and rollback readers use the same adapter.
- An initial hybrid timing attempt stopped after 56 warmups and zero retained intervals because its unsafe-u64 fixture was rejected by the baseline. That attempt is retained. The supported fallback fixture then passed eight warmup-only preflight intervals before the final run.

Evidence:

- [Consumer trace](target/performance-history2-consumer-trace.json).
- [Rejected full-envelope timings](target/performance-history2-timings.json) and [deep-input rejection](target/performance-history2b-adversarial-depth.json).
- [Hybrid timings](target/performance-history3-timings.json), [raw run](target/performance-history3-browser-probe.json), [62 browser checks](target/performance-history3-adversarial.json), [frozen build receipt](target/performance-history3-build-input.json) and [shipping input](target/performance-history3-shipping-input.json).

The updated action log adds 104 timing-distribution summaries (40 full-envelope cells plus 64 hybrid cells, each with six retained intervals), bringing it to 898 bounded observations across 254 discovered entrypoints; semantic review of 1,053 binding references remains pending. Whole-action acceptances and matched T3 wins remain zero. The final all-action timing log requested by the user is still incomplete.

## Release static asset network startup

Release SPA assets use build-time gzip sidecars. In this fixed-service browser fixture, precompression reduced the throttled fresh-start median from 69.33 s to 24.05 s and the reload median from 65.77 s to 1.08 s. Local reloads cost about 14 ms more, and the web-resource ZIP model grows by 5.31 MB. These results do not establish a matched win against any of the seven T3 targets.

### Retained measurements

Each cell has three retained observations after one warmup. Times start at navigation time origin and end after the release WASM app mounts, the chat composer is visible, six startup HTTP reply bodies succeed, three production SSE handshakes arrive, fonts load, and the command menu opens through two animation frames. Browser launch is excluded.

| Scenario | Raw median | Runtime gzip median | Precompressed median |
| --- | ---: | ---: | ---: |
| Loopback, fresh browser | 223.6 ms | 547.5 ms | 235.7 ms |
| Loopback, reload | 108.3 ms | 125.3 ms | 122.3 ms |
| 1.6 Mbit/s, 300 ms, fresh browser | 69.33 s | 24.17 s | 24.05 s |
| 1.6 Mbit/s, 300 ms, reload | 65.77 s | 1.09 s | 1.08 s |

[Raw retained samples, ranges and method](performance-static-assets-2026-11-13.json) keep every cell. Sample p95/p99 equal the maximum with three observations; they do not estimate population tails. The local fresh-start ranges overlap: 222.6–225.1 ms raw and 216.6–239.4 ms precompressed.

Compressed WASM came from the browser cache on reload; raw WASM transferred again. The WASM representation shrinks from 12,871,334 to 4,038,591 bytes. Representation size is not a transfer count on cache hits. CDP transferred-byte observations include response headers. Open SSE bytes are excluded; the separate aggregate of completed HTTP responses extends through post-boundary validation and is not total traffic through the UI timer.

### Method and tradeoffs

The study used Chrome 157.0.8103.52 on macOS arm64, Apple M2 Max, 32 GiB, at 1280×800. Six separate functional intervals passed at 390×844; this is not physical mobile testing. One warmup block and three retained blocks rotated strategy order and alternated link order. Each strategy/link pair launched a fresh browser process; reload reused that context. The OS filesystem was warm. Other user/system activity and three read-only source reviewers were present; no owned build, test, or other benchmark overlapped the timed study.

The ordinary optimized release SPA was frozen from the PR5 source tree now at main `0c140e8db1fef716c4ebb10276d91cbe6c322ae5`. The retained cohort used a prior gate's release assets with one stale, unreferenced 2,318-byte Markdown helper and its module preload. The final HTML omits it and reorders two remaining preloads. Final gated WASM and generated JavaScript binding bytes match the measured cohort exactly. Every strategy shared the same frozen inputs; the table is a controlled serving-strategy comparison, not exact final whole-action timings. A private test-only production-router fixture supplied fixed empty external-service replies and used real in-memory terminal and combined-SSE handlers. The browser remapped the baked API base to its private origin. No personal provider, capture, or filesystem endpoint ran. All 48 intervals succeeded, including 36 retained intervals. Requests already started at the clock boundary were validated afterward; that extra wait is excluded from the timer.

The impaired link used CDP at 1.6 Mbit/s download, 0.8 Mbit/s upload and 300 ms latency. Real phone CPU, radio behavior, packet loss, DNS/TLS, physical cold disk, populated histories, provider execution and multi-environment reconnect catch-up remain outside this study. T3's 0.7 MB target counts socket bootstrap metadata, and its reconnect target is not a page reload.

Runtime gzip avoids sidecar packaging but increased local fresh-start latency. Precompression uses build CPU and keeps serving cheap. All 131 regenerated sidecars have the same encoded lengths and compressed payloads as the measured prototype; the deterministic candidate uses gzip OS metadata 255 instead of 19. Both decompress to the exact originals.

The fixed ZIP-deflate6 model of final gate-generated web resources grows from 6,383,604 to 11,693,629 bytes with sidecars. This is a resource-only model, not a full signed app/update archive or an update-latency measurement. The 14 ms local reload cost and added package bytes are explicit tradeoffs for the impaired-link gains; this study does not prove that every action/client is faster.

### Build and compatibility

Trunk's release post-build hook atomically writes smaller deterministic gzip sidecars while preserving originals, modes and modification times. Debug builds skip compression. Python 3.9+ is required for the hook. The stage runs after asset pipelines and finalized staging HTML, per the [pinned Trunk pipeline](https://github.com/trunk-rs/trunk/blob/v0.21.14/src/pipelines/html.rs).

The companion server negotiates gzip only on its static fallback, retaining original MIME types, identity fallback and `Vary: Accept-Encoding` alongside CORS negotiation. API and SSE replies bypass this service. Tests cover gzip rejection, missing sidecars, HEAD metadata, byte ranges, conditional 304s, missing assets and live API/SSE handlers. Five build-hook tests cover artifact round trips, deterministic output, metadata, replacement/removal, debug/release behavior and missing staging directories.

Validation: the full 11-step merge gate passed (2,331 Rust tests, zero failures, six existing ignored tests; five Python build tests). Four functional browser flows passed with the actual gate-generated release assets at 1280×800 and 390×844. The native production router contract passed in the gate; the generated-asset UI flows used the same fixed-service fixture as the strategy study. All 130 final release sidecars decompress to exact originals.

Local full receipts remain under ignored `target/performance-network-boot1-*`, including `protocol.json`, `browser-run.json`, `timings.json`, `artifact-compatibility.json` and `code-review.json`. The committed JSON contains retained samples and receipt hashes. Failed warmup/probe attempts remain separate; no retained sample was discarded or relabeled. The broad action audit and final all-action timing log remain in progress.

Shipping: [PR #6](https://github.com/bryantleft/bruhs-ai/pull/6) merged at `aa26ef9a9a0751cde8cc376e384f8846b36f7322`; tested/merged tree `c0935dc69575da3328f55d7b0105ccd6100109e2`. The main checkout was fast-forwarded while preserving four overlapping WIP files through a checked three-way restore; all other original files, the empty index and original stashes were preserved. Owned fixture servers and browsers are stopped. The performance goal remains active.

## Resume export and callback discovery

Copying a CLI resume command now avoids converting the conversation into portable transcript data. In the mounted-app fixture, 75,000 messages took a median 1.95 ms from pointer input to Clipboard invocation before this change; the candidate median was below the timer’s observed resolution. Pointer input through Clipboard completion and two animation frames stayed near 49 ms. This removes unneeded work; it does not establish a visible latency improvement or any matched T3 win.

### Retained observations

The table shows palette-click medians with six retained intervals per implementation/case. The JSON retains all three trigger methods and every retained sample. No-session/no-active outcomes do not write the clipboard, so their invocation metric is absent.

| Data | Before: input to Clipboard call | Candidate | Before: Clipboard + two frames | Candidate |
| --- | ---: | ---: | ---: | ---: |
| empty-codex | 0.10 ms | 0.10 ms | 48.40 ms | 48.60 ms |
| small-codex | 0.05 ms | 0.00 ms | 49.05 ms | 48.35 ms |
| many-75k-codex | 1.95 ms | 0.00 ms | 48.85 ms | 48.60 ms |
| reasoning-16m-claude | 0.40 ms | 0.00 ms | 48.95 ms | 48.85 ms |
| tool-output-8m-claude | 0.20 ms | 0.00 ms | 48.50 ms | 49.20 ms |
| no-session-75k | No write | No write | 32.25 ms | 31.90 ms |
| project-fallback-75k | 1.60 ms | 0.05 ms | 48.80 ms | 49.20 ms |
| shell-quoting-small | 0.00 ms | 0.10 ms | 49.00 ms | 49.15 ms |
| no-active | No write | No write | 32.05 ms | 31.60 ms |

[Retained samples and protocol](performance-resume-export-2026-11-13.json) preserve 378 intervals: 54 warmups and 324 retained intervals across nine cases, three triggers and two implementations. Sample p95/p99 equal the maximum for six observations; they do not estimate population tails. Zero readings are below measurement resolution, not zero work.

### Boundaries and correctness

Both optimized WASM bundles mount the real app with identical private state-injection and projection-counter hooks. The active conversation is offscreen in Inbox. Data preparation, menu opening/filtering, state hashing, storage/thread loading and clipboard readback verification are outside the action clock. Implementation order alternates within each case/trigger/sample. The same warmed app/profile is reused across cases; physical cold/mobile, OS clipboard, physical paint, provider and external T3 work are unmeasured. Other user/system activity may occur; no owned build, other benchmark or chart browser overlapped the timed run. Lightweight audit/document work occurred.

Fixture-state JSON encoding for setup and verification is outside the action clock. Portable transcript projection performed by the export handler is inside the clock whenever it occurs. The Clipboard metric ends at the actual native `writeText` invocation. The completion metric ends after the real browser Clipboard promise resolves and two animation frames; no-write outcomes end after two frames. A capture microtask is not a handler-completion clock and is not used. The initial diagnostic preflight containing that mistaken metric is retained separately.

Chrome 157.0.8103.52 was identified by CDP revision `0985600c06a688b9c669e74afde7dacd5aaed047`. Its [headless startup](https://chromium.googlesource.com/chromium/src/+/0985600c06a688b9c669e74afde7dacd5aaed047/chrome/browser/headless/chrome_browser_main_extra_parts_headless.cc) selects the [platform-independent memory clipboard](https://chromium.googlesource.com/chromium/src/+/0985600c06a688b9c669e74afde7dacd5aaed047/components/headless/clipboard/headless_clipboard.cc). The test does not touch the user’s OS clipboard. The API is real; the backing store is isolated browser memory.

Every timed interval verified the expected clipboard result or absence of writes, dialog closure, no denied API requests and no console errors/warnings. Exact seeded state hashes match across implementations and before/after each trigger group. Projection counts are checked after each group’s final interval: baseline one for active threads versus candidate zero; no-active remains zero. Separate regression controls preserve Markdown and serialized T3 request bodies, and preserve the missing-project notice with no request. Those responses are mocked before transport and never write to an external T3 database.

Ordinary release validation: the full 11-step gate passed (2,331 Rust tests, zero failures, six existing ignored tests; five Python release-asset tests). Eight functional checks passed on the gate-generated bundle with a 100-message conversation restored through IndexedDB and visible in Chat. They cover all three resume-command triggers, fresh missing-session notices without clipboard writes, identical Markdown and the missing-project guard without a T3 request. Stored records stayed unchanged and private probe exports were absent. The browser revision matched the pre-run metadata; the observed minimum positive timer step was approximately 0.1 ms.

### Discovery correction

The source audit tool previously discarded all text after the first inline test module, including the production command-bar component. It also omitted event types and desktop callbacks outside `views`. It now retains whole-file event candidates, scans all desktop Rust sources and disambiguates references with character columns. Comments, test-only and lifecycle callbacks remain explicitly unclassified search candidates.

On the isolated main candidate, references increase from 1,031 to 1,093; all old references remain, including six newly recovered command-bar controls. On the working tree with existing WIP, references increase from 1,053 to 1,127. Twelve of the additional working-tree references are outside desktop views, including test-only performance code that still needs classification. These are not additional accepted user actions.

The first semantic catalogue group records 45 intents from 86 web/desktop commands. It separates opening forms from committing records, and integrated terminal creation from system Terminal launch. Downstream loading, persistence, network, error/cancel/retry, other UI controls and supported clients remain to inspect. The final all-action timing log is incomplete.

Local full receipts and private source capsules remain under ignored `target/performance-export-resume1-*` and `target/performance-action-semantics1-*`. Shipping code excludes the private hooks.


### Shipping and refreshed source snapshot

[PR #7](https://github.com/bryantleft/bruhs-ai/pull/7) merged at `015c680c05df0a13045d63a4353978a6cc57f5d8`. The complete merged tree is `1c5d71fbb3ee20b8cc8ba614b1ab1dfd63cff398`, identical to the fully gated candidate. Production source hashes match the reviewed code, and all owned fixture servers and browsers are stopped.

Local main advanced independently to `a6f72da6bda06241437c4ee6d1a603f83f637458` with five unrelated commits during the audit. Its committed and uncommitted work is preserved. The performance change is on remote main; local main has not been reset, rebased, merged or updated with it. Exact merged reports are available locally beside the ongoing action log.

A fresh working-tree scan discovers 262 source entrypoints and 1,188 binding search references. The additional routes and controls came from concurrent workspace development; they are not measured or accepted actions. The earlier 1,127-reference count above describes its frozen pre-integration snapshot. The refreshed semantic catalogue retains separate published-main and local working-tree source profiles. There remain 1,006 normalized bounded observations, zero whole-action acceptances and zero matched faster-than-T3 results. The complete all-action timing log remains unfinished and the performance goal stays active.

Concurrent reactive-agent edits arrived in three additional files after source validation. The 262-entrypoint/1,188-reference catalogue is a captured snapshot, not a claim to include later edits. The preservation receipt records their hashes without reverting them.


## Actual thread opening and pin metadata

The row-directory-only candidate is rejected after 84 ordinary-release opening intervals. The mixed 41,000-part median is 231.85 ms on baseline and 239.85 ms on the candidate. No general opening improvement is established. Its preserved patch/source and six-case results are in [the thread-opening report](performance-thread-open-2026-11-13.md). A full merge gate was not run for that unadopted candidate.

A separate published-main diagnostic proves that pinning replaces the existing transcript and composer. The isolated identity/presence-memo fix passes 56 real pin/unpin intervals and retains the sampled message/composer nodes and focus. On the 460-message/41,000-part fixture, pin medians improve from 146.2 to 49.95 ms and unpin from 129.2 to 38.8 ms, including real local commit, two frames and final-row layout. The [mount-state report and samples](performance-chat-mount-2026-11-13.md) preserve the scope and controls. Nine additional ordinary-release flows, two native tests and three code reviews pass. PR #8 merged at `8350d9b7303c050f7ebf843e7465eb04291d31ee`; the full 11-step gate passes 2,333 Rust and five Python tests, with zero failures and six existing Rust ignores. The merged tree `1c2c07963f189e1840a5c0f8299c1b31b995e783` equals the frozen successful-gate tree. Nine functional flows also pass on the gate-generated assets.

The growing timing log now includes 1,026 normalized observations from 212 partial evidence records. The complete all-action inventory/coverage and matched T3 results remain unverified. No new network-byte, impaired-link or server-store acceptance follows from these local UI clocks. External host compilation is an explicit limitation; no owned build or benchmark overlaps retained data.

The initial gate hit ENOSPC in a debug-WASM dependency fingerprint. All other steps passed. Unused Rust incremental object intermediates were removed while preserving source, measurement binaries and raw data, and the required full gate then passed. Both attempts and the cleanup receipt remain archived. Measured and gate-generated application bytes match; HTML differs only in modulepreload ordering, with normalized equivalence and final runtime checks.

All owned fixture servers and browsers are stopped. Local main remains at its concurrent committed state; no reset, stash, merge or rebase touched it. The broader action/network goal and final complete all-action timing deliverable remain active.

The final source snapshot for this pass discovers 273 entrypoints and 1,231 binding references. Eleven routes and additional callbacks arrived through concurrent workspace work since the previous scan; discovery is not measured acceptance. Source profiles retain published main `8350d9b7` and the separate local working-tree hashes.


## Conversation save snapshots withheld, 2026-11-13

The candidate shares immutable conversation snapshots across the raw-original cache and save queue. It preserves v12 storage, value equality including NaN, mutable display recovery, coalescing, in-flight edits and exact-original checks. It remains unmerged.

The 224-interval actual-WASM study retains six samples per case/format/version cell. Prepared 41,000-part, 45,000-message and 75,000-message save-loop medians improve by 18.25, 18.90 and 30.70 ms. Primary short-history completion is mixed. A separate 396-interval follow-up retains 32 samples per small-case/format/version cell; prepared non-finite fallback median completion is 9.80 ms on main and 26.30 ms on the candidate. Keep the change withheld until the short-history behavior is explained or improved. The loop's yield before noticing an empty queue is an investigation target; causal attribution is not established.

All 620 measured saves pass complete canonical/count/float/cache-original checks. Twenty functional controls cover coalescing, edits during commits, conflicts and abort/retry. Three source reviews, the full 11-step gate (2,337 Rust passes, six existing ignored; five Python passes), and nine ordinary release-app flows pass. Correctness does not override the performance decision. An initial stale shared Cargo artifact and failed JS Map-summary preflight were rejected before retained measurement; their receipts remain.

These clocks cover local enqueue and checked IndexedDB commit/acknowledgement/queue drain, including existing scheduler yields. Load/recovery, caller draft construction, verification and restoration are outside clocks; the main app is unmounted for timings. No whole-action, UI/paint, cold-I/O, mobile, provider/network or matched T3 acceptance is established. Ordinary send request types carry prompt/attachments/session options rather than the stored history; this source check establishes no network latency win.

The reviewed report is `docs/performance-conversation-snapshots-2026-11-13.md`; provenance is in its JSON companion. Exact sources, bundles, distributions and decisions are retained under `target/performance-snapshot1-*`. The living action log now has 276 discovered source entrypoints, 1,245 binding references pending semantic review, 214 partial records and 1,114 normalized observations. Whole-action acceptances and matched T3 wins remain zero. The three added source entrypoints are concurrent weekly-report HTTP routes, not work from this candidate.

Next: measure commit acknowledgement separately from idle queue drain and address any unnecessary empty-queue yield without weakening coalescing or conflict guards. Bounded history reads, incremental persistence, network measurements and semantic all-action coverage remain required. Published main remains at `8350d9b7`; no PR or merge was created in this pass. Local main advanced concurrently to `c543e865` (15 ahead, four behind published main); its commits, index and other edits were retained. The performance goal remains active.


## Conversation queue snapshots and idle-frame removal merged, 2026-11-13

PR [#9](https://github.com/bryantleft/bruhs-ai/pull/9) is merged at `53c1f1a365d32d2a93f0831aa5ce2ddaf8a029f6`. The merged tree `353ba3ab01feb4df122d97d46fcb47ec3c0bc7c9` matches the final commit, all ten reviewed/gated production files and both read-back documents. The change combines immutable conversation snapshots with an early empty-queue exit before another scheduler yield. Batch selection remains after the initial yield; stored v12 data, value equality including NaN, display recovery, exact originals and grouped transaction guards remain intact.

Natural-entry local queue-drain medians improve in all eight prepared-history workloads (six retained samples per cell). For 41,000 parts, 153.85 becomes 111.20 ms; for 75,000 messages, 258.80 becomes 224.95 ms. Every successful candidate single edit requests one frame, compared with two on main8350. The 396-interval focused cohort uses 32 samples per small history/format/version cell, eight in each requested calibrated frame phase. Its small-history drain medians also improve. Entry-to-commit readings remain variable and have unfavorable cells; short after-yield write work stays similar. This is local queue/IndexedDB evidence with the app unmounted, not whole-action/UI/paint/provider/network or T3 acceptance.

All 620 comparison saves pass complete canonical/count/float/original, exact-clock and frame checks. Forty untimed browser control intervals pass. Three independent source reviews and a fresh document readback pass. The full eleven-step gate passes 2,337 Rust tests (six existing ignored) and five Python asset tests. Nine checks on the generated ordinary release app pass. Both Vercel host checks were successful before the head-matched squash merge. The connector's create action returned integration403; authenticated Git/gh completed the already-authorized publication.

The separate 204-interval old-main/Arc-only diagnostic records transaction completion before Rust callbacks and distinguishes later acknowledgement/idle drain. Its capture-order invariant and three-clock bridge pass. An exploratory late-listener study and failed capture-flag preflight remain archived; their commit-boundary numbers are excluded. The earlier snapshot1 withholding decision remains historical evidence and is not rewritten as a win.

Reports are `docs/performance-conversation-save-2026-11-13.md` and its JSON companion. Exact raw studies, source capsules, binaries, gate logs and merge/readback receipts are retained under `target/performance-save-boundary*` and `target/performance-save2-*`. The action log now retains 276 discovered source entrypoints, 1,245 binding references awaiting semantic review, 217 partial records and 1,382 normalized observations. Whole-action acceptances and matched T3 wins remain zero.

Published main is updated. The local main's concurrent commits, index and WIP are preserved; its diverged checkout has not integrated the remote performance merges. No root code reset, stash, rebase or merge was performed. Next work remains bounded/incremental history access, real action completion and network/device variants, and expansion of the semantic action catalogue beyond its first palette group. The full performance goal remains active.

### 2026-11-13: Original-envelope comparison prototype remains unmeasured

No new application timing or matched T3 result was established in this pass. Published main remains `53c1f1a365d32d2a93f0831aa5ce2ddaf8a029f6` (PR #9).

The isolated prototype compares a stored parts-v1 envelope with the expected header and content projection before rebuilding typed messages. It retains strict outer-field validation and the existing typed decoder for every mismatch. It adds no cache, schema, or dependency. Sixteen native storage tests passed, including differential checks for metadata, content, malformed envelopes, float/status equality, and existing snapshot behavior. Browser compatibility and performance remain unverified, so the change was not merged.

The planned private WASM comparison would cover eight synthetic histories, both legacy and formatted records, and five save-stage clocks against published main. Zero browser timing samples were collected. Trunk stalled before compilation; a process sample showed only `_dyld_start`. A byte-identical copy also timed out on `--version`, and Cargo subsequently timed out on `--version`. Those diagnostic timeouts are build-tool observations, not application latency measurements. The stalled owned processes were stopped. No benchmark browser or server was started.

The all-action timing log remains incomplete: 276 source entrypoints, 1,382 normalized observations of partial variants, and no whole-action or matched T3 acceptances. The first semantic catalogue covers 45 command intents; it does not enumerate every possible application action. Network transport, provider startup/streaming, device conditions, and cold/error variants remain part of the goal. Unknown timings remain unknown. See [performance-action-log.md](performance-action-log.md) and [performance-action-catalogue.md](performance-action-catalogue.md).

The tested candidate source and reconstructible patch are preserved under `target/performance-envelope1-production-source/` and `target/performance-envelope1-production.patch`. Input/source hashes, native-test receipt, private probe source, tool diagnostics, and cleanup receipts use the same prefix. Inactive incremental compiler caches were removed to recover space; pinned binaries, source capsules, raw evidence, and root application work were preserved.

Resume from the pinned main revision and production patch after build tools launch normally. Verify both WASM implementations and the browser differential controls before collecting timings. A supported improvement still requires the full repository gate, source review, and ordinary release checks before merging.

### 2026-11-13: Direct-envelope trial completed; candidate withheld

Build tools recovered. The corrected release-WASM trial completed 224 saves (192 retained, six per cell), 21 codec controls and 20 functional saves. Seventeen native storage tests pass, including a reproduced and fixed empty-checkpoint normalization difference. Long writer-prepared histories have lower observed medians, but tool-heavy and legacy results vary. The prototype remains unmerged; full-history reads and rewrites remain. See [performance-conversation-envelope-2026-11-13.md](performance-conversation-envelope-2026-11-13.md).

The action log retains 1,542 normalized observations and 218 partial records. A source-pinned supplement identifies 16 unmeasured network entrypoints in unpublished root work outside the frozen 276-entrypoint catalogue. The generator stops at the new router composition; a complete current-source refresh and semantic review are still required. Whole-action and matched T3 acceptances remain zero.

All owned browsers/builds/server jobs are terminal, and the temporary checkout/branch were removed after verifying patch reconstruction. Root application files were not edited. Evidence, source capsules and cleanup receipts use `target/performance-envelope3-*`. Next: bounded history access and incremental persistence, then actual UI/network actions and matched T3 workloads. The goal stays active.
