# Workbench execution acceptance

This checks the real server, HTTP API, Codex SDK, child processes, Git worktrees
and durable task/usage stores using an owned synthetic Codex executable.
It catches execution failures the small coding/chat task bench does not reach.
No model account, API key, forge write or real application data is used.

Requirements: macOS or Linux, Python 3.9+, Git and a built Bruhs server.

```sh
CARGO_INCREMENTAL=0 cargo build -p bruhs-server --locked
python3 bench/workbench/run.py --server target/debug/bruhs-server
```

The full `scripts/gate.sh` builds the server and runs these scenarios. It reads
the executable path from Cargo's artifact output, including a configured target
directory. Quick mode retains the unit and benchmark-integrity checks and does
not run this server scenario.

| Scenario | Required outcome |
| --- | --- |
| Completion | A completed Codex process becomes a Completed task with its final answer, session and artifact retained. Progress text does not replace the final answer. |
| Failed exit | `turn.completed` followed by a nonzero process exit is Failed. |
| Truncated output | A clean exit without a turn completion receipt is Failed. |
| Stalled exit | A CLI that reports completion and closes stdout but fails to exit within the SDK's existing grace period is stopped and Failed. |
| Delegation | Parent and child finish in separate real worktrees with distinct provider sessions and artifacts. The original checkout stays untouched. |
| Cancellation | Stop marks the task Cancelled and its owned provider process exits. |
| Recovery | Graceful host restart restores an active task as Interrupted without replay; its previous provider has exited. Completed answers, sessions and artifacts survive reopening. Explicit continuation resumes its saved session and completes. |
| Usage | Completed tasks and failed exits that reported usage each retain exactly one record with correct uncached/cache/output counts. The usage API reports the expected model, aggregate counts and unpriced status. |

Each run creates a new directory under `target/workbench-evaluation/` containing
the copied executable, synthetic home, repositories, provider invocations,
server log and `report.json`. `--output` chooses a new directory; existing ones
are refused. `--timeout` sets each operation's observation budget, defaulting
to the established 60-second Workbench request deadline. Status sampling is
functional observation, not an input-to-paint or model latency measurement.

The child server receives a fresh environment with only fixture configuration
and private stores. Its PATH contains the owned Codex executable, and the fake
provider writes only inside the synthetic home. The driver waits for that
server's new startup log before calling its port. The fixture is trusted test
code, not an OS sandbox for arbitrary generated code. Cleanup stops the owned
server and any surviving process whose command still identifies this fixture.

All eight planned cases remain in the report. Startup, cleanup, missing cases
or source changes cause failure. The report hashes the copied executable and
the scoped source inputs. Source hashes alone do not prove an older executable
was built from that checkout; retain the build command/log for a source-bound
checkpoint. Earlier failing reports remain separate.

The Codex wrapper now emits the common Result event only after the SDK stream
settles successfully with a turn completion and a usable session. A later
process failure overrides usage; an interrupted stream cannot produce success.
This also lets the existing usage recorder retain successful Codex turns.
Claude success results are likewise held until process settlement; a later
failed exit or missing completion becomes an error. Both native SDKs report
failure when their shared process helper has to terminate a CLI that closes
stdout but will not exit.

This suite does not measure model intelligence, live Codex authentication,
complete billing/descendant costs, real PR delivery, browser/native UX,
forced host crashes or physical devices. Those remain part of the
[competitive evaluation goal](../../docs/competitive-evaluation-goal-2026-11-15.md).
