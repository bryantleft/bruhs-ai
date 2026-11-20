# Live delivery checks

`read_github.py` checks whether the real server and installed GitHub CLI read
the repository selected by `origin` and retain that identity in a review
proposal. It can reproduce a mismatch caused by a saved `gh` default pointing
to another remote. Both repositories need an existing PR with the chosen
number; their contents need not match.

```sh
CARGO_INCREMENTAL=0 cargo build -p bruhs-server --locked
python3 bench/delivery/read_github.py \
  --server target/debug/bruhs-server \
  --output "$HOME/.cache/bruhs-delivery/$(date -u +%Y%m%dT%H%M%SZ)" \
  --origin cli/cli --other bryantleft/bruhs-ai --number 10
```

Use repositories the authenticated `gh` can read. The example includes a
private repository; substitute an accessible second repository for another
account. The fixture creates its own local Git repository and private server
stores. Its `gh` wrapper retains the original authentication home because the
native keyring lookup needs it. No credentials are copied or logged. The
wrapper logs nonsecret repository selectors and refuses write verbs, explicit
API methods, body fields and input files before the installed CLI starts.

Six checks cover inherited repository/host selectors, a saved default that
differs from origin, REST placeholder scope, agreement between the displayed
PR and frozen proposal, an unexecuted proposal after restart, and the CLI
read-only guard. Empty comment lists cannot prove API scope, so the fixture
also inspects the selectors delivered to the native child process. The driver
retains both failures and missing planned checks. It exits nonzero on a failed
case, source changes, startup failure or cleanup failure.

The copied executable, installed CLI version/hash, source snapshots, raw read
responses, local preview, process observations and report remain in the output
directory. A source snapshot describes the checkout observed by the runner;
retain the build log for its corresponding binary. A retained baseline binary
can be run through the same driver without overwriting earlier evidence.

These checks use real authenticated `github.com` reads and synthetic local
contents. They submit no PR, review, comment, push or merge, and run no coding
model. Enterprise hosts and mutation destination binding use isolated CLI
fixtures in `delivery::repository_tests`; complete live delivery, CI/review
feedback and competitive comparisons require additional evidence.

Run the offline process and guard checks without GitHub access:

```sh
cargo test -p bruhs-core delivery:: --lib --locked
python3 -m unittest discover -s bench/tests
```

## Source-review guard

The offline profile uses a private Git repository and real delivery HTTP routes.
It prepares two untracked files, changes their payload boundaries, then checks
that execution stops before a Git write. Desktop and mobile receipts explain
that a new review is required. Every Git write and forge command is blocked by
the fixture before execution. No authenticated forge access or model runs.

```sh
BRUHS_PLAYWRIGHT_PATH=/path/to/playwright python3 -m bench.delivery.source_guard \
  --server target/debug/bruhs-server --output target/fresh-source-guard
```

Use a fresh output directory. A retained old server can reproduce the bypass
with `--legacy-negative-control --skip-browser`; it reaches a blocked Git add
without changing HEAD or the index. The
[source-review checkpoint](../../docs/evidence/delivery-source-fingerprint-2026-11-16.json)
retains both runs and the native mutation controls. These checks do not prove
an atomic source snapshot or live forge delivery.
