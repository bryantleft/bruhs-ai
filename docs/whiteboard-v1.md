# Whiteboard v1

Bruhs now has its own Rust whiteboard on web and desktop. Boards belong to a working folder, save locally, and let an agent propose editable diagrams for the owner to review. The tldraw SDK is confined to the benchmark comparator.

## Use it

Choose **New whiteboard** in Command K. `R`, `E`, `A`, `T`, `N`, `F`, and `P` select rectangle, ellipse, arrow, text, note, frame, and freehand tools. `V` selects and `H` pans. Press Enter to create with the current shape tool or edit a selected shape. `C` creates the next node and a bound arrow. Tab focuses the next shape; arrow keys move the selection. Command K also exposes grouping, alignment, distribution, snapping, undo, redo, copy, paste, import, export, and source checks.

A board's **Link selection** action stores typed links to threads, project files, tasks, PRs, meetings, people, or documents. File links retain their line ranges. Boards reopen from their project's catalog. Web saves use IndexedDB; desktop saves use the app profile's private whiteboards directory. These are independent local stores. Portable JSON moves a board between them.

## Agent authority and living diagrams

Boards start private. **Share with agents** grants access to that board from threads in its working folder. The app answers `list_whiteboards`, `read_whiteboard`, `propose_whiteboard`, and `whiteboard_proposal_status`. Reads are paginated and tool replies are bounded. Proposals are unavailable in read-only turns. The broker supplies the caller's thread and turn; tool arguments cannot substitute another identity.

A proposal contains exact operations sealed to the board's revision and contents. The owner sees before and after images plus changed shapes, source metadata, and exact operations. Only the owner UI accepts or rejects it. Acceptance saves the candidate before adopting it in the editor. An ordinary edit makes pending proposals stale; another tab's revocation immediately blocks cached agent reads and discovery. A save conflict preserves the draft and offers a recovery export.

A Mermaid block in chat opens as editable shapes with a thread backlink after a private-copy preview. A module-diagram proposal can name a relative source file; the app fingerprints it within the calling project. Opening the board, focusing the window, or choosing **Check source drift** flags changed or unavailable sources. Request a new agent proposal to refresh the diagram and review that change before accepting it.

## Interchange

SVG, PNG, portable JSON, and supported Mermaid diagrams export locally. Mermaid flowcharts and basic sequence diagrams import as shapes; a selected frame or group can export to Mermaid. Legacy import supports tldraw 5.6.0's format-1/schema-2 documents and Excalidraw version 2. The preview lists conversion losses before saving a new private copy. Unsupported schemas or unsafe content produce an explicit error. Import never fetches remote images; embedded raster data passes size, dimension, digest, and aggregate decode-budget checks.

The shared model handles geometry, hit testing, bound arrows, hierarchy, snapping, inverse-operation undo, and clipboard remapping. Web draws with Canvas 2D. Desktop uses GPUI paths and cached text, with local raster layers for rotated text and images. Pointer movement previews avoid cloning the whole document on each event.

## Verification

The production browser harness exercises Command K, keyboard and pointer editing, bindings, file backlinks, reload and project reopen, undo/redo, Mermaid, real legacy imports, exports, owner previews, source drift, stale proposals, cross-tab revocation, conflicting writers, and recovery copies. Additional cases exercise real touch pointers and editing a loaded board with the network offline. The combined Chat-to-board test also guards deferred scroll callbacks after Chat unmounts.

The final optimized bundle passes **74 checks total**: 37 at 1280×900, 18 at 640×480, and 19 at 390×844, with no runtime errors, console errors, or unexpected network requests. A separate touch run passes **19 checks** under 400 ms latency and 50 KiB/s upload/download shaping, including real touch drawing, reload, and loaded-board offline editing. Evidence: [browser validation](../bench/whiteboard/results/2026-11-14/validation.json), [throttled validation](../bench/whiteboard/results/2026-11-14/network-3g-validation.json), [native validation](../bench/whiteboard/results/2026-11-14/native-validation.json), and [source/build identity](../bench/whiteboard/results/2026-11-14/build-source.json).

Native functional acceptance uses the dev build in a real GPUI window with a separate synthetic project and app profile. It covers immediate keyboard focus, ten connected nodes, a frame, pointer ellipses, Cmd+Enter text submission, undo/redo, a file backlink with lines 1–2, and catalog reopen. A production-painter test verifies actual local image decoding and PNG pixel output. Twenty-seven model tests and five core tests pass. Strict Clippy passes for all targets in core, models, server, web, and desktop, including the optional benchmark features; the final Chat lifecycle guards also pass a fresh strict web check.

## Rendering comparison

The frozen primary acceptance **passes** on an Apple M2 Max in Chrome 157.0.8103.52. Across 12 independent pairs, the geometric mean Rust/tldraw completed-frame interval ratio is **0.0902**, with a percentile 95% interval of **0.0879–0.0921**, wholly below 1. Each member equally weights the three workload means. The ratio is the geometric mean of 12 pair ratios; the interval uses 10,000 pair-level bootstrap resamples with seed 20261114. Lower intervals indicate higher completed compositor throughput.

The table reports arithmetic means of every member’s workload mean, in milliseconds. The 10,000-shape results are descriptive, based on two pairs.

| Cohort | Renderer | Pan | Single node | Whole selection |
| --- | --- | ---: | ---: | ---: |
| 1,000 · unthrottled · 12 pairs | rust | 0.881 | 0.874 | 1.167 |
| 1,000 · unthrottled · 12 pairs | tldraw | 2.242 | 4.062 | 26.120 |
| 1,000 · normal vsync · 3 pairs | rust | 8.347 | 8.333 | 8.320 |
| 1,000 · normal vsync · 3 pairs | tldraw | 8.333 | 8.333 | 27.164 |
| 10,000 · unthrottled · 2 pairs | rust | 2.310 | 2.385 | 6.924 |
| 10,000 · unthrottled · 2 pairs | tldraw | 14.214 | 47.549 | 384.721 |

Normal-vsync panning and single-node dragging are effectively tied at this display’s refresh ceiling. Whole-selection dragging differs. Completed-frame throughput is distinct from pipeline latency: in the primary cohort, mean pipeline latency for pan/single/group was 15.940/15.647/15.400 ms in Rust and 17.582/18.531/49.486 ms in tldraw. These are instrumented pipeline durations, not measured input-to-photon latency.

The replay confirms common scene records, node origins, arrow-terminal geometry, and final movement. Output is not pixel-identical: text baselines, arrowhead fill, SDK arrow occlusion, watermark, and selection adornments differ. Bruhs uses one aggregate outline for multiple selection; tldraw shows individual indicators. Those production design differences remain part of this interaction comparison. The result does not isolate equal drawing commands or a renderer algorithm.

The real-window native probe used the dev build at a reported 1280×801 viewport and 2× scale. Mean display-link callback intervals for pan/single/group were 8.732/8.332/8.333 ms at 1,000 shapes and 15.687/17.361/36.043 ms at 10,000. The larger whole-selection case remains a native scaling limit. These scheduling numbers are not release-build or GPU benchmarks.

Machine-readable [results and raw-artifact hashes](../bench/whiteboard/results/2026-11-14/summary.json) retain all cohort members. Raw compressed traces and replay screenshots remain under `target/whiteboard-benchmark/`; the `completion.json` files verify that measured artifacts stayed unchanged. `bench/whiteboard/report.py` reproduces the aggregate JSON from those retained cohorts.

The frozen [protocol](../bench/whiteboard/protocol.md) compares the same neutral 1,000- and 10,000-shape scene inputs with pinned tldraw 5.6.0, common local font bytes, real bound-arrow geometry, fresh Chrome launches, alternating order, and retained Chromium traces. The acceptance statistic uses 12 independent 1,000-shape pairs. Normal-vsync and 10,000-shape cohorts remain separate. The v1 performance pass criterion is the 1,000-shape browser comparison; native GPU capture is excluded from that pass decision. Native callback intervals characterize scheduling; the two failed Metal System Trace pilots leave native GPU timing inconclusive.

## V1 boundaries

The acceptance above covers a local editable diagramming workflow and a narrowly measured rendering comparison. It does not establish superiority over every whiteboard or general agent diagram accuracy. There is no CRDT, automatic device synchronization, hand-drawn style, or automatic meeting-summary/transcript backlink generation. Imported custom shapes, advanced Mermaid constructs, and unsupported image transforms are rejected or shown as explicit conversion losses. File line ranges are retained and labeled; the current file viewer opens the file without automatically highlighting that range.
