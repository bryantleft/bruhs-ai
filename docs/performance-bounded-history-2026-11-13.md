# Bounded conversation storage experiment

The one-message-per-row layout is withheld. Large-history page reads and incremental writes improve, while complete history reads and some small reads regress. No format or schema change from this experiment has merged. The performance goal remains active.

The private optimized WASM fixture compares published main `53c1f1a3` storage calls with indexed ranges and revision-checked mutations in one binary. Chrome 157.0.8103.52 ran on an Apple M2 Max, macOS 25.3.0. Both versions use writer-prepared records; the Infinity fixture retains structured fallback. These are warm local storage component clocks. Application mounting, TaskSaves scheduling, input/paint, physical cold I/O, providers and network completion are excluded.

## Repeated operation timings

Values are **baseline / indexed median milliseconds**, with six retained intervals after one warmup per cell. Case, operation and implementation order reverse on alternating blocks. Ranges and all samples remain in the JSON timing receipt. A page requests 77 messages, which is a different unit from T3 timeline items. Earlier-page cases require more than 77 messages.

| Synthetic history | Final page | Earlier page | Full history | Append | Replace final message |
| --- | ---: | ---: | ---: | ---: | ---: |
| small-control | 0.30 / 0.90 | 0.30 / 0.40 | 0.30 / 1.00 | 0.55 / 0.30 | 0.60 / 0.50 |
| mixed-41k-parts | 57.20 / 11.25 | 57.30 / 10.75 | 55.05 / 63.05 | 111.85 / 0.50 | 114.50 / 1.20 |
| messages-45k | 66.70 / 1.10 | 62.55 / 1.10 | 63.20 / 355.00 | 116.00 / 0.55 | 114.65 / 0.60 |
| messages-75k | 107.35 / 1.15 | 108.75 / 1.10 | 103.55 / 606.50 | 197.10 / 0.50 | 199.00 / 0.55 |
| large-tool-output | 15.70 / 11.60 | 13.85 / 3.60 | 14.90 / 14.75 | 25.00 / 0.60 | 26.20 / 1.15 |
| long-reasoning | 2.50 / 2.70 | N/A | 2.50 / 2.70 | 5.70 / 0.50 | 6.00 / 5.90 |
| escaped-output | 5.25 / 6.50 | N/A | 5.35 / 6.30 | 12.25 / 0.50 | 12.40 / 0.85 |
| structured-fallback | 0.55 / 0.95 | 0.50 / 0.45 | 0.50 / 1.15 | 1.10 / 0.40 | 1.10 / 0.40 |

The 75,000-message full read rises from 103.55 ms to 606.50 ms. Its final-page read falls from 107.35 ms to 1.15 ms. The mixed 41,000-part fixture contains 460 messages; it is not a 41,000-message history. Single large bodies still dominate affected-row work: replacing the long reasoning message remains about 6 ms. Six samples describe this run and do not establish population tails.

## Initial preparation cost

Each value below is one complete warm legacy load, encoding and atomic sidecar preparation. These are **single intervals**, outside repeated read/write clocks. The private experiment retains the original blob alongside indexed rows. They do not establish a repeated migration distribution or a physical cold-I/O cost.

| History | Messages | Preparation ms (n=1) |
| --- | ---: | ---: |
| small-control | 100 | 5.90 |
| mixed-41k-parts | 460 | 170.40 |
| messages-45k | 45000 | 1132.70 |
| messages-75k | 75000 | 1946.30 |
| large-tool-output | 100 | 38.80 |
| long-reasoning | 2 | 8.20 |
| escaped-output | 50 | 20.10 |
| structured-fallback | 100 | 3.90 |

## Correctness and measurement limits

All 532 executed intervals have matching message/part counts, canonical content and exact float-bit readback, with terminal transactions complete before the second Rust timer call. Read clocks start before the storage call and include database/request setup, decoding, range validation and owned projection through completed Window construction. Mutation clocks start after expected-head/original loading and caller draft construction, and include encoding, transaction setup, checks and writes. Both end before summary/result marshaling, content/float hashing and restoration. The exact matrix audit accepts 76 cells over seven blocks, including 456 retained intervals. The raw runner still records exit 1 and pass=false: its final assertion expected 546 because it forgot that both the two-message reasoning fixture and the 50-message escaped fixture have no earlier page. The original report and samples are preserved; a separate audit documents the count correction.

Four schema control groups cover upgrade, blocked/versionchange, forced failure rollback and store/index collision. Thirteen data control groups cover range identity, stale revisions, parent deletion, occupied append keys, missing/corrupt rows, abort/retry, non-finite originals, empty history and concurrent preparation. A format-barrier control reproduces same-version legacy/indexed divergence and verifies that a later database version closes the old writer. Twenty functional API invocations pass.

Indexed mutations write one message row and one head row. They check the head revision and affected originals, and leave the unread prefix unchanged. This changes the consistency protocol: every official writer must advance the revision atomically. The private full-read probe validates the declared interval, but does not audit unexpected rows outside it. Full snapshots, exports and deletion need that check before production use.

Three reviews cover only the two-file schema preparation. Its ordinary build passes all 11 gate steps (2337 native tests; 6 existing ignored). That gate ran after private reader/writer hooks were removed. Ordinary release UI controls were not run because this schema and layout are withheld. This is not production acceptance of the prototype.

## Next work

Measure a chunked layout that keeps bounded page access and incremental writes without the full-read penalty. Derive chunk budgets from measurements. Before activation, cover every reader/writer, import, offscreen queue, rewind, export, parent deletion and rollback; preserve recovery and corruption handling; and enforce a version barrier against legacy writers. Native desktop persistence remains separate work.

The action log also needs complete semantic actions, real UI completion, provider/network paths and all seven equivalent T3 workloads. This study establishes no matched T3 win.

## Evidence

- [Raw samples](target/performance-bounded3-browser-probe.json)
- [Exact-matrix audit](target/performance-bounded3-run-audit.json)
- [Normalized timings](target/performance-bounded3-timings.json)
- [Source/asset/harness receipt](target/performance-bounded3-run-receipt.json)
- [Schema-only gate](target/performance-bounded3-schema-gate-receipt.json)
- [Remaining integration requirements](target/performance-bounded3-integration-requirements.json)
