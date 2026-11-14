# Conversation chunk-size sweep

No fixed message-count chunk size is adopted. Large chunks recover full-history performance but lose incremental-write gains on content-heavy histories. Small chunks keep bounded work while adding overhead to complete reads. The next experiment needs to account for content size. The performance goal remains active.

The optimized private WASM fixture compares published-main `53c1f1a3` whole-record storage with 1, 16, 128 and 1,024 messages per indexed row in one binary. These are exploratory parameters, not production budgets. Chrome 157.0.8103.52 ran on Apple M2 Max/macOS 25.3.0. All partial pages request 77 messages, a different unit from T3 timeline items.

## Repeated operation medians

Each cell has **three retained intervals after one warmup**, with alternating case/operation/implementation order. Values below are milliseconds. Three samples describe this sweep; they do not establish population tails or a generally optimal size. All sample ranges remain in the timing JSON.

| History | Operation | Baseline | 1 msg/row | 16 msgs/row | 128 msgs/row | 1,024 msgs/row |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| small-control | tail | 0.40 | 0.80 | 0.40 | 0.40 | 0.40 |
| small-control | earlier | 0.30 | 0.40 | 0.40 | 0.40 | 0.40 |
| small-control | full | 0.30 | 1.20 | 0.50 | 0.40 | 0.50 |
| small-control | append | 0.80 | 0.50 | 0.50 | 1.00 | 1.00 |
| small-control | replace | 0.90 | 0.50 | 0.50 | 1.00 | 1.20 |
| mixed-41k-parts | tail | 58.10 | 11.20 | 10.40 | 23.20 | 54.50 |
| mixed-41k-parts | earlier | 56.90 | 10.70 | 8.90 | 14.50 | 57.20 |
| mixed-41k-parts | full | 54.50 | 64.30 | 53.30 | 54.00 | 54.60 |
| mixed-41k-parts | append | 110.50 | 0.60 | 3.50 | 18.60 | 110.80 |
| mixed-41k-parts | replace | 111.40 | 1.40 | 3.80 | 19.70 | 115.70 |
| messages-45k | tail | 62.80 | 1.10 | 0.60 | 0.60 | 1.80 |
| messages-45k | earlier | 64.70 | 0.90 | 0.40 | 0.50 | 1.80 |
| messages-45k | full | 64.50 | 352.50 | 95.90 | 81.20 | 61.40 |
| messages-45k | append | 117.50 | 18.10 | 0.60 | 1.00 | 3.20 |
| messages-45k | replace | 117.80 | 0.70 | 0.50 | 1.00 | 3.30 |
| messages-75k | tail | 114.20 | 0.90 | 0.50 | 0.50 | 0.80 |
| messages-75k | earlier | 107.70 | 1.00 | 0.60 | 0.60 | 0.80 |
| messages-75k | full | 103.70 | 583.00 | 158.80 | 136.60 | 102.90 |
| messages-75k | append | 199.90 | 0.60 | 0.60 | 1.40 | 1.50 |
| messages-75k | replace | 202.00 | 0.60 | 0.50 | 1.20 | 1.60 |
| large-tool-output | tail | 14.10 | 12.60 | 12.40 | 13.50 | 16.00 |
| large-tool-output | earlier | 13.80 | 3.40 | 7.30 | 13.50 | 13.80 |
| large-tool-output | full | 14.00 | 14.80 | 13.90 | 14.00 | 13.90 |
| large-tool-output | append | 26.90 | 0.60 | 1.50 | 21.90 | 21.30 |
| large-tool-output | replace | 25.00 | 1.20 | 1.60 | 22.90 | 21.80 |
| long-reasoning | tail | 2.70 | 2.90 | 2.80 | 3.00 | 3.00 |
| long-reasoning | full | 2.60 | 2.90 | 2.80 | 2.70 | 2.80 |
| long-reasoning | append | 5.80 | 0.50 | 5.50 | 5.90 | 5.80 |
| long-reasoning | replace | 6.00 | 6.20 | 6.00 | 6.00 | 6.10 |
| escaped-output | tail | 5.40 | 6.50 | 5.60 | 5.60 | 5.60 |
| escaped-output | full | 5.30 | 6.70 | 5.60 | 5.50 | 6.30 |
| escaped-output | append | 12.40 | 0.60 | 1.10 | 11.70 | 11.70 |
| escaped-output | replace | 12.50 | 0.80 | 1.10 | 11.50 | 11.70 |
| structured-fallback | tail | 0.60 | 1.00 | 0.60 | 0.60 | 0.60 |
| structured-fallback | earlier | 0.50 | 0.50 | 0.40 | 0.60 | 0.60 |
| structured-fallback | full | 0.60 | 1.30 | 0.70 | 0.70 | 0.70 |
| structured-fallback | append | 1.10 | 0.40 | 0.50 | 1.30 | 1.30 |
| structured-fallback | replace | 1.20 | 0.40 | 0.40 | 1.20 | 1.10 |

For 75,000 messages, 1,024-message rows read the complete history in 102.9 ms versus 103.7 ms on the baseline. But on the mixed 460-message/41,000-part fixture, those rows append in 110.8 ms versus 110.5 ms on the baseline; 16-message rows append in 3.5 ms. No single tested size retains every gain. The unfavorable 45,000-message, one-message-row append cell remains visible rather than being filtered.

## Initial preparation (n=1 per history and size)

These 32 values are single complete warm sidecar preparations, outside repeated read/write clocks. They retain the original legacy blob. They are not repeated migration distributions or physical cold-I/O measurements.

| History | 1 msg/row ms | 16 msgs/row ms | 128 msgs/row ms | 1,024 msgs/row ms |
| --- | ---: | ---: | ---: | ---: |
| small-control | 5.90 | 1.70 | 1.50 | 1.00 |
| mixed-41k-parts | 169.40 | 153.90 | 159.70 | 202.90 |
| messages-45k | 1138.00 | 233.40 | 177.20 | 177.20 |
| messages-75k | 1951.90 | 390.60 | 299.60 | 354.50 |
| large-tool-output | 38.00 | 37.60 | 40.00 | 42.70 |
| long-reasoning | 9.00 | 9.40 | 8.60 | 8.80 |
| escaped-output | 17.60 | 20.40 | 18.50 | 18.60 |
| structured-fallback | 4.10 | 1.70 | 1.70 | 1.70 |

## Boundaries and correctness

Read clocks include database/request setup, decoding, strict range/row/cardinality checks and owned projection through the completed Window and terminal transaction. Complete reads also count every by_thread row, so extra rows outside the declared interval are rejected. Mutation clocks include encoding and parent/revision/affected-original checks through one chunk write, one head write and commit completion. Expected-head/original loading and caller draft construction happen first. Summary/result marshaling, hashing, verification and restoration are outside clocks.

The app is unmounted. TaskSaves scheduling, actual UI/input/paint, physical cold I/O and provider/network completion are excluded. Writer-prepared hybrid records are used when eligible; the non-finite fallback stays structured. Indexed mutations change the consistency protocol: all official writers must update the revision. Unread chunks are not globally compared. Neighboring messages in the affected chunk are loaded fresh and retained.

All 760 intervals pass content/count/order/float-bit checks and exact timer/transaction event checks. The exact 190-cell matrix has 570 retained intervals. Four schema groups, 16 data-control groups, 49 range-edge pairs, 60 functional pairs and the format-barrier control pass. No owned builds or other timing studies overlapped retained samples; unrelated system activity was uncontrolled.

The first sweep stopped after one warmup because its full-record hash compared different per-size thread IDs. That failed run remains unchanged. The corrected harness validates each full record against its own thread ID and compares message content plus exact float bits across sizes. Cross-size preflight and the complete matrix pass. The compiled binary was reused unchanged for this harness correction.

## Integration status

No layout or unused schema upgrade from these experiments has merged. Only the private WASM check/build and the stated browser controls validate this prototype. The earlier 11-step ordinary schema-only gate does not validate these readers/writers. Every application consumer, native persistence, actual UI/provider/network flow and equivalent T3 workload still needs its own evidence.

## Evidence

- [Timings and all samples](target/performance-chunk5-timings.json)
- [Exact matrix and raw receipt](target/performance-chunk5-run-receipt.json)
- [Source/build/harness input](target/performance-chunk5-input.json)
- [Failed first comparison](target/performance-chunk4-browser-probe.json)
