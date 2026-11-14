# Whiteboard action coverage and existing timing evidence

This checkpoint adds 66 source-reviewed whiteboard intent requirements and extracts 60 timing groups from existing rendering evidence. It also fixes the generic action-log renderer so regeneration preserves all 40 previously published response-body observations. No benchmark, build or UI run was launched for this checkpoint.

The successor catalogue contains 128 intent requirements, preserving the earlier 62 records and three source profiles exactly. These requirements describe what must be measured. **No complete whiteboard action or matched T3 comparison is accepted.**

## Action requirements

The source capture contains 43 files. The shared catalogue declares 35 board actions and ten tools. Every board action resolves through either the shared model or an explicit web/native dispatch arm. Together these 45 declarations and 21 additional intents form the 66 new requirements. The additional intents cover pointer gestures, keyboard selection, dialogs, imports, proposal review, navigation and lifecycle transitions.

Tool selection and drawing have different completion boundaries. Importing to a preview and saving that preview are separate intents. Mutating actions must include the exact persisted revision and rendered feedback. Browser handoff, browser-owned download completion and native file-write completion need separate clocks; export completion requires the resulting bytes to be verified. A declared callback alone proves neither supported behavior nor speed.

[All 128 intent requirements](performance-action-catalogue.md) retain completion boundaries, required variants, source hashes and line anchors. The source capture is non-atomic and predates the landing. A later observation records changes to the shared server router, web API and global command bar; the capture stays historical. The latest pinned broader discovery remains 376 source entry points and 1,383 binding references; neither count is a complete semantic denominator. Global chat entrypoints, remaining fields/menus, downstream views and other product families still require review.

## Existing evidence retained

The owner already measured the production Rust painter through the opt-in `whiteboard-benchmark` feature against pinned tldraw 5.6.0. The aggregate replay reproduces every field except its new creation timestamp, including all 121 retained artifact hashes. The original twelve-pair decision also reproduces exactly. No measurements were rerun.

| Extracted groups | What each sample means | Boundary retained |
| --- | --- | --- |
| 54 browser groups | One member's workload mean | Completed compositor-frame interval, unique-feedback interval or pipeline duration during scripted pan/single/group movement. |
| Six native groups | One display-link callback interval | Correlated scheduling samples from a native dev probe; one run per scene. |

The browser groups keep the twelve-pair primary cohort, three-pair normal-vsync cohort and two-pair scaling cohort separate. Quantiles describe member workload means, not individual frames or population latency tails. Metrics reuse underlying runs. The original arithmetic means and bootstrap decision remain distinct from these quantiles.

The pages share scene geometry, local fonts and bound-arrow checks, while selection adornments and some rendering details differ. The comparator omits its toolbars. Scripted renderer API updates omit the full app host, pointer dispatch and durable save. Normal-vsync pan/single movement is effectively tied at the display ceiling. Native GPU timing remains inconclusive; the original owner report records two failed pilots. These results do not establish input-to-photon latency, complete UI/store performance, native-versus-browser superiority or a T3 Code win.

The 37 source files common to this capture and the owner's recorded build identity match exactly, including all eight web whiteboard modules. That establishes source correspondence for those files, not a rebuilt current-main application. [Original protocol](../bench/whiteboard/protocol.md) · [Owner report](whiteboard-v1.md) · [Replay and extraction check](../target/performance-whiteboard-actions1/existing-benchmark-check.json).

## Source findings to investigate

| Path | Source-confirmed work | Next required proof |
| --- | --- | --- |
| Web pointer movement | The handler calls `host.edit`; a no-gesture move returns without a model edit, but the host still notifies and the canvas effect clears/redraws the scene. | Count real production UI draws for idle hover versus active gestures, then measure a candidate that preserves drawing, pan, cancellation and errors. |
| Web source checks | Each source fingerprint HTTP request is awaited before the next starts. | Verify request/concurrency behavior and realistic source/error/revision cases before choosing batching or bounded parallelism. |
| Project catalogue | Web loads the entire summary index before filtering. Native enumerates and loads all board documents before filtering by project. | Separate project-local versus global board counts, payload sizes, warm/cold storage, corrupt records and first usable catalogue feedback. |

These are structural findings without a measured user-impact estimate or a production whiteboard fix in this checkpoint. Whiteboard-owned correctness work remains with its owner.

## Log preservation repair

The released generic renderer omitted response-body fields when invoked directly. The new standalone tool reads the normalized body files, verifies their raw pointers and hashes, rejects duplicate/orphan/mismatched observations, and retains their historical order. A full replay preserves all 2,236 earlier timing groups, 34 request-count cases and 40 body observations exactly. Six separate corrupt-input controls reject before writing output.

The finalization coordinator adopted the audit’s tested source and committed the standalone tool as `dbf91478ead14df7595e1c233bbd680c9588ee06`. The later source observation records the authorized landing and three storage-source deltas without changing older captures. The audit made no Web/model/Cargo graph change or Git/index write.

The private log successor contains 246 partial evidence records and 2,296 typed timing groups, including the 60 historical groups extracted here. Request cases remain 34 and body observations remain 40. The catalogue expands to 128 source-reviewed requirements; complete-action acceptances and matched T3 wins remain zero. Published root docs and the ignored full ledger are unchanged by this checkpoint. [Log successor](performance-action-log.md) · [Preservation receipt](../target/performance-whiteboard-actions1/log-extension-check.json).

Fresh root performance measurements still need their own resource coordination and source/build observation. The prepared HTTP paired study has not launched. All seven equivalent T3 workloads and the final complete action-timing log remain open.
