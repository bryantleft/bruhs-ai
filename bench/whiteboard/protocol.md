# Whiteboard rendering protocol

The acceptance test compares the production Rust canvas painter with tldraw 5.6.0 on one machine. It tests compositor throughput during scripted edits, not physical display refresh or superiority over every whiteboard.

## Frozen retained run

Use Chrome 157.0.8103.52 on the Apple M2 Max with a 1280 by 800 CSS-pixel viewport and device scale factor 1. Both pages use the same local Monaspace Argon variable font bytes, 18-pixel text, background, shape geometry, short labels, straight bound arrows and scene JSON. The fixture contains exactly 1,000 or 10,000 shape records: half rectangles, a quarter text and a quarter arrows, with two bindings per arrow. tldraw keeps its default culling and caches; its page limit increases to 20,000 solely to admit the 10,000-shape scene. It runs in a production React build without its toolbars. No production Bruhs entry imports the SDK.

The primary cohort has 12 independent pairs at 1,000 shapes, alternating Rust-first and tldraw-first. Chrome launches afresh for every member. The primary cohort uses Chromium's `--disable-frame-rate-limit` switch, with a separate three-pair normal-vsync cohort. A two-pair 10,000-shape cohort characterizes scaling and is not part of the v1 acceptance decision. Run members sequentially, with no builds or other benchmark jobs owned by this task active during measurement. Other applications on this user's machine remain open; record thermal state and hardware without claiming a laboratory isolation level.

Each member loads and validates the entire scene, awaits the renderer load API and document.fonts.ready, then waits for two requestAnimationFrame callbacks, and performs a 500 ms unmeasured pan warmup before restoring the initial camera. Each measured workload lasts two seconds and follows the same time-based linear trajectory:

1. Pan the camera from the origin to (400, 200).
2. Reset the camera and drag the first rectangle by (180, 90).
3. Reset the camera and drag the entire selection by (180, 90).

Keep the first-node edit when starting the whole-selection drag. Finish each workload at the exact endpoint outside the measured marks. Validate final camera, all node and text coordinates, arrow bindings and endpoint geometry after each workload. The tldraw adapter clamps normalized exact arrow anchors 0.001 inward; allow at most 0.25 CSS pixels of terminal deviation, while requiring exact node coordinates. Screenshots belong to a separate replay outside retained timing, including a final whole-selection image. Scene validation failures invalidate the member and remain in the evidence. A failed or invalid member stops the cohort. Keep its evidence; any fix requires a new directory and a complete new cohort. Do not replace a slow valid run.

## Trace interpretation and decision

Collect Chromium benchmark, compositor, GPU, user-timing and frame events identically for both pages. Identify the page renderer PID from its workload marks. Match `PipelineReporter` begin/end events by process and async identity. Retain only `STATE_PRESENTED_ALL` reporters with unique source/sequence pairs. A completed frame must start and complete between 200 ms after its workload start and 100 ms before its end. These fixed margins exclude setup and trailing work. Require at least two retained completed frames and two distinct feedback timestamps per workload; a zero or nonfinite member score invalidates the cohort; fail if expected events are absent or this minimum is unmet; never substitute JavaScript callback counts.

For each workload sort reporter end timestamps and compute the mean successive interval (last minus first, divided by count minus one). Count distinct completed compositor frames even when batched presentation feedback gives several the same timestamp. Also report intervals between unique feedback timestamps, completed-frame counts, JavaScript update counts, dropped reporters and normal-vsync results so throughput is not confused with physical scanout. A frame's reporter duration is end-to-end pipeline latency and is reported separately from throughput.

A member's primary score is the equally weighted mean of its three workload means. The decision statistic is the geometric mean of the 12 paired Rust/tldraw score ratios. Bootstrap the independent pairs, with seed 20261114 and 10,000 resamples, to obtain a percentile 95% interval (sorted resample indices 249 and 9749). Accept the narrowly scoped completed-frame interval claim only if that entire interval is below 1. Report per-workload results, including ties or regressions, rather than hiding them in the aggregate. The unthrottled cohort measures renderer capacity without the display refresh ceiling; it supports only the compositor-throughput claim. Normal-vsync ties are ties, and neither cohort establishes input latency or physical display smoothness.

Keep raw traces compressed, the scene and font hashes, exact build hashes, browser/GPU details, protocol hash and every run summary. Pilot runs identified the trace schema and fixed label wrapping and local assets; they are excluded from the retained cohort.

## Native spike

Render both fixtures through the production GPUI painter in an isolated real window. Record geometry and display-link callback intervals, labeled as scheduling measurements. Two pilot attempts to launch the probe under Metal System Trace ended with SIGKILL and produced no usable process-filtered GPU or presentation rows. Preserve those failures and report native GPU timing as inconclusive. Native CPU scene time, GPU execution and presentation are distinct measurements. No native-versus-browser win is inferred from scheduling or CPU scene timing.
