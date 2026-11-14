# Resume export and callback discovery audit

Copying a CLI resume command now avoids converting the conversation into portable transcript data. In the mounted-app fixture, 75,000 messages took a median 1.95 ms from pointer input to Clipboard invocation before this change; the candidate median was below the timer’s observed resolution. Pointer input through Clipboard completion and two animation frames stayed near 49 ms. This removes unneeded work; it does not establish a visible latency improvement or any matched T3 win.

## Retained observations

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

## Boundaries and correctness

Both optimized WASM bundles mount the real app with identical private state-injection and projection-counter hooks. The active conversation is offscreen in Inbox. Data preparation, menu opening/filtering, state hashing, storage/thread loading and clipboard readback verification are outside the action clock. Implementation order alternates within each case/trigger/sample. The same warmed app/profile is reused across cases; physical cold/mobile, OS clipboard, physical paint, provider and external T3 work are unmeasured. Other user/system activity may occur; no owned build, other benchmark or chart browser overlapped the timed run. Lightweight audit/document work occurred.

Fixture-state JSON encoding for setup and verification is outside the action clock. Portable transcript projection performed by the export handler is inside the clock whenever it occurs. The Clipboard metric ends at the actual native `writeText` invocation. The completion metric ends after the real browser Clipboard promise resolves and two animation frames; no-write outcomes end after two frames. A capture microtask is not a handler-completion clock and is not used. The initial diagnostic preflight containing that mistaken metric is retained separately.

Chrome 157.0.8103.52 was identified by CDP revision `0985600c06a688b9c669e74afde7dacd5aaed047`. Its [headless startup](https://chromium.googlesource.com/chromium/src/+/0985600c06a688b9c669e74afde7dacd5aaed047/chrome/browser/headless/chrome_browser_main_extra_parts_headless.cc) selects the [platform-independent memory clipboard](https://chromium.googlesource.com/chromium/src/+/0985600c06a688b9c669e74afde7dacd5aaed047/components/headless/clipboard/headless_clipboard.cc). The test does not touch the user’s OS clipboard. The API is real; the backing store is isolated browser memory.

Every timed interval verified the expected clipboard result or absence of writes, dialog closure, no denied API requests and no console errors/warnings. Exact seeded state hashes match across implementations and before/after each trigger group. Projection counts are checked after each group’s final interval: baseline one for active threads versus candidate zero; no-active remains zero. Separate regression controls preserve Markdown and serialized T3 request bodies, and preserve the missing-project notice with no request. Those responses are mocked before transport and never write to an external T3 database.

Ordinary release validation: the full 11-step gate passed (2,331 Rust tests, zero failures, six existing ignored tests; five Python release-asset tests). Eight functional checks passed on the gate-generated bundle with a 100-message conversation restored through IndexedDB and visible in Chat. They cover all three resume-command triggers, fresh missing-session notices without clipboard writes, identical Markdown and the missing-project guard without a T3 request. Stored records stayed unchanged and private probe exports were absent. The browser revision matched the pre-run metadata; the observed minimum positive timer step was approximately 0.1 ms.

## Discovery correction

The source audit tool previously discarded all text after the first inline test module, including the production command-bar component. It also omitted event types and desktop callbacks outside `views`. It now retains whole-file event candidates, scans all desktop Rust sources and disambiguates references with character columns. Comments, test-only and lifecycle callbacks remain explicitly unclassified search candidates.

On the isolated main candidate, references increase from 1,031 to 1,093; all old references remain, including six newly recovered command-bar controls. On the working tree with existing WIP, references increase from 1,053 to 1,127. Twelve of the additional working-tree references are outside desktop views, including test-only performance code that still needs classification. These are not additional accepted user actions.

The first semantic catalogue group records 45 intents from 86 web/desktop commands. It separates opening forms from committing records, and integrated terminal creation from system Terminal launch. Downstream loading, persistence, network, error/cancel/retry, other UI controls and supported clients remain to inspect. The final all-action timing log is incomplete.

Local full receipts and private source capsules remain under ignored `target/performance-export-resume1-*` and `target/performance-action-semantics1-*`. Shipping code excludes the private hooks.
