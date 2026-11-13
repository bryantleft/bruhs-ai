# Preserve chat while changing thread metadata

Pinning and unpinning keep the existing transcript and composer mounted. The view now depends on selected-thread identity and empty/live state; header metadata and message changes continue through their existing reactive children. Navigation, deletion and empty-state transitions still replace the appropriate view.

On the synthetic 460-message, 41,000-part history, median pin completion falls from 146.2 to 50.0 ms and unpin from 129.2 to 38.8 ms. These clocks include the real local commit and two animation frames.

| History | Action | Baseline median (range), ms | Candidate median (range), ms |
| --- | --- | ---: | ---: |
| small-control | pin | 56.20 (55.10–59.50) | 39.40 (38.50–52.10) |
| small-control | unpin | 54.25 (51.40–56.10) | 38.65 (37.00–50.70) |
| mixed-41k-parts | pin | 146.20 (136.90–151.40) | 49.95 (40.50–56.30) |
| mixed-41k-parts | unpin | 129.20 (125.80–142.90) | 38.80 (24.40–47.30) |

There are 56 intervals: eight warmups and 48 retained, six per cell. Separate Chrome processes/profiles use alternating implementation and case order. The [JSON companion](performance-chat-mount-2026-11-13.json) retains every retained sample and exact input/source hashes. Sample p95/p99 equal the maximum; six observations do not establish population tails.

Timing starts at the real default keyboard shortcut with the composer focused, and ends after the normal thread-store transaction commits, two animation frames pass and final-row layout is read. Bootstrap, fixed-service HTTP/SSE startup, history opening, draft preparation and validation are outside the action clock. Both bundles are ordinary release WASM, with no private application exports.

Every interval checks one committed thread write, zero message writes, the intended pin flag, unchanged stored messages, transcript text and unsent draft. Full saved-thread values match across implementations. All 24 retained candidate actions keep the sampled original message wrapper, composer node and composer focus; the baseline replaces both nodes in every retained action. No action-triggered HTTP fetch is observed; transport bytes were not captured.

Nine additional functional flows pass: six full-history/empty-thread/return transitions, expanded work and thinking preserved through pin, a renamed header updated without replacing chat, and active-thread deletion. Two native tests cover memo suppression and navigation/deletion/empty/live predicates. Three independent source reviews report no issues.

The tests use desktop headless Chrome, real IndexedDB and synthetic histories behind fixed private services. Physical paint/mobile, OS-cold storage, other clients/triggers, conflict/error/cancel/retry, providers and impaired-link behavior remain unverified. Other host activity may occur: unrelated Cargo/rustc processes were observed before the retained run, none afterward. No owned build or other benchmark overlaps retained data.

This is measured pin/unpin variant evidence. It establishes no T3 comparison or full-action/all-action completion. The wider performance and network goal remains active.
