# Original-envelope comparison, 2026-11-13

The prototype remains unmerged. It reduces the sampled write interval for long writer-prepared message lists, while tool-heavy workloads are flat or slower. All histories still read and rewrite in full. Published main remains `53c1f1a365d32d2a93f0831aa5ce2ddaf8a029f6` (PR #9).

## What was measured

Private, unmounted release WASM in Chrome 157 on macOS arm64. Eight synthetic histories, legacy and writer-prepared records, two implementations, one warmup and six retained samples per cell: 224 saves, 192 retained. Case, format and implementation order alternate. Both versions request one pre-write scheduler frame. No owned builds or other measurements overlap retained intervals; unrelated system/user activity is not controlled.

Five clocks cover synchronous enqueue, early IndexedDB completion, post-commit acknowledgement/drain, total worker completion, and first pre-save scheduler callback through early completion. The early observer registers when the native transaction is created, before Rust handlers. Per-run checks reproduce the three Rust clock reads and verify event order. Loading/recovery, caller draft construction, verification/hash/restoration and teardown are outside timers. UI/paint, cold physical I/O, provider and network completion are excluded.

## Writer-prepared results

Milliseconds, medians. After-yield columns also show observed minimum–maximum. Six observations do not establish population tails. The JSON evidence retains all legacy variants and every clock/range. Non-finite records remain structured fallback.

| History | After-yield main | After-yield candidate | Commit main / candidate | Drain main / candidate |
| --- | ---: | ---: | ---: | ---: |
| 100 messages | 0.60 (0.60–1.70) | 0.65 (0.50–1.90) | 2.55 / 1.40 | 2.55 / 1.40 |
| 460 messages / 41,000 parts | 112.75 (108.00–131.80) | 110.50 (102.40–127.50) | 113.05 / 114.50 | 117.00 / 118.45 |
| 45,000 messages | 126.70 (117.60–143.30) | 114.80 (111.00–118.50) | 127.75 / 115.35 | 131.75 / 119.25 |
| 75,000 messages | 215.40 (207.90–242.90) | 195.95 (189.00–204.00) | 215.65 / 201.05 | 222.05 / 207.45 |
| Large tool output | 25.65 (24.10–28.00) | 26.75 (23.70–38.40) | 25.85 / 26.95 | 28.80 / 30.00 |
| Long reasoning | 6.25 (5.90–6.70) | 6.15 (5.90–7.20) | 6.45 / 6.40 | 7.00 / 6.90 |
| Escaped output | 12.50 (12.00–13.70) | 12.95 (12.30–17.20) | 12.75 / 13.30 | 13.95 / 14.50 |
| Non-finite fallback | 1.05 (0.90–1.20) | 0.95 (0.90–1.00) | 8.50 / 1.55 | 8.50 / 1.55 |

Entry-to-commit and drain include variable initial scheduling. For example, the non-finite fallback has similar after-yield work despite very different entry medians. Legacy 45,000-message after-yield medians are 212.45 ms on main and 221.70 ms on the candidate. Do not infer a universal improvement from the long prepared-list cells.

## Correctness and decision

The candidate compares a strict stored envelope with the expected canonical header and content projection. Mismatches use the full typed decoder. Three independent source reviews identified one typed-input difference in two passes: `Some("")` checkpoints normalize to `None` when decoded. A direct Rust-constructed regression fails before the eligibility guard and passes after. Current UI reachability is not established because snapshot IDs are validated. Empty checkpoints now use the existing decoder.

Seventeen native storage tests pass. Twenty-one untimed browser codec controls and twenty functional saves pass, including malformed/extra/missing fields, equivalent headers, special numbers, deep inputs, coalescing, edits during commit, stale originals and abort/retry. All 224 comparison saves pass canonical/count/float/cache-original, exact-clock, frame and event-order checks. These checks do not establish ordinary release UI behavior or a full merge-gate pass for this candidate.

The prototype adds comparison work and another codec path without removing full-history I/O. The mixed result does not justify adoption now. Its source, patch and evidence remain available; bounded reads and incremental persistence are the next architectural work.

## Action and network coverage

The frozen all-action log has 276 source entrypoints, not a complete semantic inventory. A current-root refresh stops at a new router composition in unpublished root work. A source-pinned supplement identifies nine delivery HTTP routes and seven routes on the optional paired-task listener in unpublished root work. They are outside the frozen inventory and remain unmeasured. Their presence requires a generator update and consistent source refresh before any completion claim.

The log now retains 1,542 normalized observations and 218 partial evidence records. Whole-action acceptances and verified matched T3 wins remain zero. All seven screenshot workloads, real provider/device/network flows and semantic action coverage remain open.

All local evidence uses `target/performance-envelope3-*`; the companion JSON pins source and raw inputs. The earlier unmeasured/unfixed phases remain under `performance-envelope1-*` and `performance-envelope2-*`. Missing ignored artifacts require regeneration from pinned source and protocol.
