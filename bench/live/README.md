# Live coding pilot

This driver compares installed Codex with the real Bruhs server's Codex path
using the same model, reasoning effort, starting repositories and task prompts. It
runs `rootcause-paging` and `concurrency-keyed-jobs` once per entrant. These
fixtures check fixes across an entire documented contract, beyond the visible
bug report. A pilot can expose execution or coding gaps; four attempts cannot
establish a competitive ranking.

A later [runtime audit](../../docs/evidence/repository-coding-campaign-2026-11-16.json)
found automatic skill catalogs varying between four and 58 skills in the
retained pilot, despite ignored user configuration. Its correctness receipts
remain useful, but its context was not fully controlled. For a configuration
that disables those catalogs and audits actual rollouts, use the
[repository campaign](../repository/README.md). This pilot driver keeps its
historical defaults.

Build the server, then declare a per-attempt budget before starting:

```sh
CARGO_INCREMENTAL=0 cargo build -p bruhs-server --locked
python3 bench/live/run.py \
  --server target/debug/bruhs-server \
  --output "$HOME/.cache/bruhs-live/$(date -u +%Y%m%dT%H%M%SZ)" \
  --model gpt-6.1-sol --effort low --timeout 300
```

Use a model supported by the authenticated Codex installation. This pilot
requires macOS Seatbelt and an existing Codex `auth.json`. It does not start
engineering delegates. The provider processes are the products being tested.

Both entrants use `workspace-write` with approval policy `never`, no command
network access, a private home and Codex state, and ignored user configuration
and execution rules. A common wrapper fixes reasoning effort and refuses an
unsandboxed invocation. macOS cannot nest Seatbelt sandboxes: the provider uses
Codex's native sandbox, while the independent grader runs in a separate
Seatbelt process after the provider finishes. Hidden checks and reference fixes
stay outside each task workspace and prompt; the native provider sandbox does
not deny every host read. This is a local diagnostic, not an adversarial
evaluation with inaccessible graders.

The server/process isolation comes from the Workbench acceptance helper. Its
fake provider is replaced with the authenticated native CLI before the server
starts. Bruhs keeps its normal response instructions and app tools; their
stores contain only this campaign's synthetic data. Those product differences
are part of the comparison. A temporary credential symlink is removed during
cleanup; credentials are not copied into the report or snapshot.

Each campaign retains frozen fixtures and grader inputs, source hashes, binary
hashes, CLI version, provider session logs, process exits, independent check
logs, patches and all planned attempts. The grader restores pristine visible
tests alongside hidden tests, rejects generated symlinks, blocks networking,
denies user-data reads and restricts writes to its temporary directory. A pass
requires both successful provider completion and passing independent checks.
The grader also retains a raw completion receipt and a host-validated
`<check-log>.result.json` beside each check log. A clean process exit alone is
insufficient: every frozen test must start, finish and succeed, with no skips,
expected failures or missing identities. An empty suite fails before dispatch.
This is execution accounting within a shared Python interpreter, not adversarial
attestation.
Missing usage and money stay unknown. The report is updated after every attempt
and failed or unstarted attempts remain visible. A nonzero driver exit can mean
a product failure or a harness error; inspect the report before interpreting it.

This pilot has fixed entrant order, one attempt per cell and two internal
fixtures. It makes no claims about external products, statistically reliable
speed differences, monetary cost, PR delivery or connected daily work. Keep
aborted harness runs alongside valid campaigns, with the reason recorded, and
use fresh campaigns to verify changes rather than overwriting earlier results.

The test inventory comes from literal top-level unittest classes and methods
in the frozen visible and hidden files. Dynamic or inherited test layouts need
an explicit inventory contract before this pilot can accept them. Colliding
visible/hidden module names and existing output artifacts are rejected to
preserve coverage and earlier attempts.

Submitted Python and the tests still share an interpreter. The receipt detects
premature exits and skipped coverage; it is not protection against arbitrary
in-process tampering. The [completion checkpoint](../../docs/evidence/live-grader-completion-2026-11-16.json)
retains the reproduced false passes, a retained replay driver, rejection after the fix and regrading
of the six earlier live submissions. Regrading adds no new provider attempts.

The task bench now uses the same frozen-test inventory and completion receipts
through `bench/lib/test_receipts.py` and `bench/lib/test_runner.py`. The live
pilot keeps its Seatbelt process wrapper in `bench/live/grading.py`; the
separate task bench remains a local execution path. The [shared completion
checkpoint](../../docs/evidence/task-bench-grader-completion-2026-11-16.json)
records both-path regression checks and another recheck of the retained live
submissions. Historical results keep their original versions and scope.

The grader and sandbox boundaries can be checked without provider calls:

```sh
python3 -m unittest discover -s bench/tests
```

`effort.py` separately checks the real API/SDK/CLI path against a retained
baseline executable. Both get a low CLI default and an API request for max;
the baseline must record low in its provider rollout, and the fixed server
must record max. This verifies control propagation, not higher coding quality:

```sh
python3 bench/live/effort.py --baseline-server /path/to/retained/server \
  --server target/debug/bruhs-server --model gpt-6.1-sol \
  --output "$HOME/.cache/bruhs-live/effort-$(date -u +%Y%m%dT%H%M%SZ)"
```

The reusable attempt adapter also supports the separately declared
[constrained Claude repository cohort](../repository/cron-pause/README.md#constrained-claude-cli-cohort).
The default live pilot remains Codex-only. Claude's new
noninteractive fixture declines every permission request; it does not change
application permission defaults or silently grant tool access. Its five local
adapter regressions run with `python3 -B -m unittest bench.live.test_permissions`.
