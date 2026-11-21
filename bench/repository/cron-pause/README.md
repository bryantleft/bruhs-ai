# Go scheduler campaign

This fixture tests concurrent per-entry pause/resume in a pinned public Go
scheduler through installed Codex and Bruhs's Codex API/SDK path. The current
version includes a clock regression missed by the first campaign. The
[original checkpoint](../../../docs/evidence/repository-go-campaign-2026-11-16.json)
keeps version 1, supplemental review and assisted repair separate. The
[fresh version-2 checkpoint](../../../docs/evidence/repository-go-v2-campaign-2026-11-16.json)
records Codex 3/3 and Bruhs 2/3 completed, checked and reviewed attempts. All six
code submissions pass the strengthened checks; one Bruhs provider run timed out.
Neither experiment establishes an overall product ranking.

The task requires synchronous controls, preserved entry and wrapper identity,
correct running/stopped behavior, fresh-clock resume without replay, concurrency
safety, relevant tests and user documentation. The pinned source and MIT license
are in [source.json](source.json); the contract is in [prompt.md](prompt.md).

## Reproduce

Use macOS Seatbelt, Python 3.11+, Git, the recorded Go 1.25.3 toolchain, an
installed signed-in Codex CLI and a built Bruhs server. The repository has no
third-party Go dependencies. Preparation and grading invoke no model.

1. Download the archive URL in source.json with certificate verification enabled.
   Preparation verifies its pinned SHA-256 before extracting it.
2. These commands prepare and run current version 2, rather than replaying the
   archived first experiment. Prepare a fresh fixture:

   ```sh
   python3 -m bench.repository.go_fixture \
     --definition bench/repository/cron-pause \
     --archive /absolute/path/upstream.tar.gz \
     --output /absolute/path/prepared-fixture
   ```

3. Run its predeclared campaign, which makes real provider calls:

   ```sh
   CARGO_INCREMENTAL=0 cargo build -p bruhs-server --locked
   python3 -m bench.repository.run \
     --server target/debug/bruhs-server \
     --fixture /absolute/path/prepared-fixture \
     --output /absolute/path/fresh-evidence
   ```

4. Review every captured patch against the contract and execute submitted test
   suites separately. Automatic success remains provisional until the review
   accepts relevant tests, documentation and behavior beyond selected assertions.
   Retain the operator's decision; this review is unblinded.

The current [declaration](campaign.json) fixes six cells, alternating order,
GPT-6.1 Sol at high effort, 600-second provider/grading budgets and zero
corrective feedback within an attempt. Both paths are provided the same Go wrapper and offline settings. Coding
commands selected workspace or unique temporary caches; wrapper use and child
tool environment were not uniformly enforced. Independent grading pins the Go
binary and always starts with a new private cache. The fresh model-controlled version-2 campaign used these frozen settings. It is separate
from the earlier assisted-repair profile and preserves all six planned cells.

The [archived version-1 definition](versions/1/campaign.json) can be prepared with
`--definition bench/repository/cron-pause/versions/1`. The checkpoint also links
the original frozen helper snapshot for reproducing its exact grader behavior.
New model runs need fresh evidence directories and may produce different code.

## Independent checks

A settled submission is copied into a fresh sandbox. Entrant test files are
removed, then pinned upstream/public tests and withheld checks are restored.
The current suite runs 48 upstream top-level tests, two public smoke tests and
16 withheld checks with the race detector. Every frozen top-level test and every
observed subtest must start and pass once, without skips; the package and process
must finish successfully. The corrected reference passes all 66 top-level tests
and 27 nested subtests.

In the fresh version-2 campaign, all six submissions pass 66 frozen checks and
27 nested cases, with their source accepted in unblinded operator review. All
six supplied suites pass separately, including 59 added top-level tests in
aggregate. Codex completed 3/3 attempts; Bruhs completed 2/3. Bruhs repeat 3
reached the 600-second provider budget without a native final answer or
completion event. Passing later checks does not convert that timeout to success.
No retry or corrective feedback was used.

All six original attempts passed version 1's 65 checks. All their submitted
suites pass too, including 58 added top-level feature tests in aggregate across the six submissions. Supplemental review found a
stale-clock timer in the original reference and one Bruhs submission: after a
slow Schedule.Next calculation, an unrelated overdue job received another stale
countdown. The other five submissions pass that post-campaign probe.

The original automatic scores remain 3/3 per entrant. Separate review accepts
five submissions and marks one for repair. The new probe was designed after
code inspection; it cannot establish a statistically supported ranking. A
fresh-session repair on a copy of the reviewed submission, given one operator
message with the root cause, passes all
66 checks and its 61 supplied tests. It leaves the original source unchanged and
does not turn the original submission's post-campaign quality failure into an
unassisted success.

Untouched feature absence and deliberate startup, history and clock defects are
rejected. The first reference control failed on a macOS sandbox path alias;
that failed control and corrected runs remain retained. Independent grading and the provided Go wrapper use a private telemetry
off-mode file without changing the user's settings. Contract-only
inventories accelerate deliberate-defect controls; live scoring uses the full
frozen inventory. Submitted tests never increase its score.

## Evidence and limits

The runner retains inputs, grading helpers, source/binary fingerprints,
transcripts, test events and patches. Actual rollouts must match model, effort,
approval and sandbox/network settings and contain no automatic skill catalog.
A mismatch stops the campaign with every planned cell retained. Unknown billed
cost remains unknown.

Bruhs retains its normal system contract and app tools backed by private stores.
Those intentional differences prevent isolating the coding harness alone.
Grading denies network and reads of the actual user's home. Go event accounting
is not adversarial attestation; provider sandboxing is not a hidden-check secrecy
test. Model familiarity with this public repository is unknown.

One additional feature and language do not establish market leadership or a
speed/cost advantage. No Capy, T3 Code, product-default comparison, physical capture or PR
publication ran in those Codex cohorts. The separate Claude profile below
uses a different model and constrained tool configuration. The
[competitive goal](../../../docs/competitive-evaluation-goal-2026-11-15.md)
remains open.

## Constrained Claude CLI cohort

`claude_campaign.py` separately compares installed Claude Code with Bruhs's
Claude SDK/HTTP path. It uses the same pinned v2 repository task, Sonnet 5.5 at
high effort, three repeats per entrant, paired order AB, BA, AB and 600-second
provider/grading budgets (A is Claude Code; B is Bruhs). The declaration is [claude-campaign.json](claude-campaign.json).
It tests coding and transport under a constrained CLI profile, not product
defaults. Do not aggregate its scores, timing or tokens with the GPT/Codex cohorts.

Both paths receive safe mode, restricted file tools, explicit
Read/Write/Edit/Glob/Grep/Bash, no skills/Chrome/session persistence and an
enabled Bash sandbox without allowed command-network domains. Existing host
OAuth/keychain identity supplies authentication; credentials are not copied.
The common wrapper retains those settings. Bruhs's host system contract remains
an intentional difference. Coding-child environments and cache use are not
fully attested; independent grading uses the pinned Go binary and a fresh cache.

The noninteractive permission policy grants no requests. Native Claude print
mode returns denials; the Bruhs fixture responds to every permission request
with `approved=false`, without a reason or coding guidance. Denials have local
receipts containing request identity, tool and input hash. The original driver
left Bruhs prompts unanswered and produced three timeouts; that entire six-cell
campaign remains invalid comparison evidence. The corrected policy is declared
before a fresh cohort and never edits its original outcomes.

With existing Claude Code authentication and the pinned archive already
available, use a copied immutable server executable and a new output directory:

```sh
python3 -B bench/repository/claude_campaign.py \
  --server /absolute/path/frozen-bruhs-server \
  --output /absolute/path/new-claude-evidence \
  --go /absolute/path/pinned-go \
  --archive /absolute/path/upstream.tar.gz
```

This makes real provider calls. Inspect the preflight and retain every planned
cell, error and timeout. The [Claude checkpoint](../../../docs/evidence/repository-claude-campaign-2026-11-17.json)
records the measured cohort: Claude and Bruhs each completed and passed frozen
checks, submitted suites and unblinded review in 3/3 attempts. Every cell passes
66 frozen top-level checks and 27 nested cases. The submitted suites pass
separately with 93 added top-level tests in aggregate. This small constrained
cohort establishes no product-default, speed, cost or overall ranking.

The future driver rejects mutable root Cargo build-output paths, missing actual
model evidence, and changed executable/frozen-input hashes. Those guards were
tightened after the recorded cohort; its original driver remains in the
snapshot and a separate final audit verifies its models and binary hashes. Run
supplied suites and review patches separately before accepting automatic
passes. Reported subscription session dollars are estimates; total billed
model/machine and human costs remain unknown.

Check the local fixture policy and provenance guards without provider calls:

```sh
python3 -B -m unittest bench.live.test_permissions bench.repository.test_claude_campaign
```
