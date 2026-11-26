# Repository coding campaign

This campaign tests a real upstream repository feature through installed Codex
and Bruhs's Codex path. Results are diagnostic evidence for one feature, with
three fresh attempts per entrant. This document describes the protocol; the
[checkpoint](../../docs/evidence/repository-coding-campaign-2026-11-16.json)
records results and controls. They do not establish product leadership.

A second [Go scheduler campaign](cron-pause/README.md) adds concurrent pause/resume
and a separate clock-regression review. Its archived first version, strengthened
current definition and assisted repair remain separately visible; its results
also establish no overall ranking.

The task adds durable leased delivery to Huey's SQLite queue. It combines schema
migration, atomic reservation and acknowledgment, stale-token fencing, recovery
after a consumer crash, legacy dequeue compatibility, high-level APIs, tests and
documentation. The pinned source and MIT license are recorded in
[huey-leases/source.json](huey-leases/source.json).

## Reproduce

Use macOS with Seatbelt, Python 3.11, Git, an authenticated installed Codex CLI
and a built `bruhs-server`. Preparing or grading a fixture calls no model. The
campaign command makes real provider calls using the existing sign-in.

1. Download the exact archive URL in `huey-leases/source.json`. Keep certificate
   verification enabled. The preparation command verifies its SHA-256 before
   creating the fixture.
2. Prepare the pinned fixture in a fresh destination:

   ```sh
   python3 bench/repository/prepare.py \
     --definition bench/repository/huey-leases \
     --archive /absolute/path/upstream.tar.gz \
     --output /absolute/path/prepared-fixture
   ```

3. Run the declared campaign in a fresh evidence directory:

   ```sh
   CARGO_INCREMENTAL=0 cargo build -p bruhs-server --locked
   python3 bench/repository/run.py \
     --server target/debug/bruhs-server \
     --fixture /absolute/path/prepared-fixture \
     --output /absolute/path/campaign
   ```

4. Review every captured patch against the task contract. Automatic success is
   provisional until that review confirms relevant tests, documentation and
   correct behavior beyond the selected checks. Retain a separate review record;
   review failure blocks final acceptance; raw automatic results remain retained. The campaign operator records each
   decision and its evidence in a `manual-review.json` beside `report.json`.
   This operator review is unblinded and is not an external audit.

[The campaign declaration](huey-leases/campaign.json) fixes all six cells, their
order, model, effort, budgets, success criteria and intervention policy before
model access. Its budget basis explains the three-repeat diagnostic choice and
600-second step budget; the fixed order alternates the starting entrant across
repeat pairs. Model and low effort match the earlier live pilot to expand
repository coverage without changing those settings. A product-default
comparison remains separate. All planned cells stay in the denominator, including timeouts,
harness errors and attempts that never started. Preserve a defective campaign
before correcting its harness and starting a new one.

## Independent checks

The prepared fixture inventories 209 unique upstream regression identities and
27 feature checks. Upstream test bytes are restored before submitted code is
imported. Each upstream SQLite case runs in its own process and working directory
because its unmodified teardown leaves open handles to deleted WAL files.
Selected slow tests are enabled. Optional service/backend matrices and one
unrelated FileLock case with an absolute host path are explicitly excluded in
[regressions.json](huey-leases/regressions.json).

Every process must return a complete identity receipt with every declared test
started, finished and successful, with no skips. A clean exit before completion
fails. The grader also requires changed test and documentation files; relevance
and quality require the patch review. The reference patch passes all 236 checks. Three of these checks cover finite
integers above SQLite int64. They were added after a retained, invalidated
first campaign exposed a missed contract case; they are frozen before the
second campaign starts.
The untouched source, the retained faulty submission and deliberate expiry, legacy-dequeue, queue-isolation and
bad-deserialization defects fail the corresponding feature checks.

To grade a settled submission without invoking a model:

```sh
python3 -I -S bench/repository/grade_cli.py \
  --fixture /absolute/path/prepared-fixture \
  --work /absolute/path/submitted-repository \
  --output /absolute/path/fresh-grade-output \
  --timeout 600
```

## Retained evidence and limits

The runner retains the campaign declaration, frozen fixture and grading helpers,
source fingerprints, server/CLI fingerprints, actual initial-message fingerprints, provider transcripts, all attempt
outcomes, complete grading receipts and patches relative to each initial commit.
Patch capture includes new files and committed changes without changing the
submission's Git index. A temporary credential symlink is removed during cleanup;
credential contents are not copied into evidence. Treat retained logs as local
research artifacts and inspect them before sharing.

Both entrants get the same repository, prompt, model, effort and command-network
policy. Bruhs retains its normal system contract and application tools backed by
private synthetic stores. Native CLI skill catalogs, remote plugins, native app
connectors, memories and shared daemons are explicitly disabled in both paths.
Every retained rollout is checked for unexpected skill catalogs; a mismatch
stops the campaign and retains all planned cells. Bruhs-specific instructions,
application tools and transport are intentional differences in this comparison;
they prevent isolating the effect of the coding harness alone. Generated code is graded in a separate Seatbelt sandbox without
network or access to the user's home. Tests share an interpreter with submitted
Python and are not an adversarial attestation boundary. The provider sandbox
also permits some host reads; this is not a test-secrecy evaluation.

One public repository may overlap model training. This benchmark excludes external products,
product-default campaign or complete personal-workflow delivery is measured.
Token counts do not establish billed cost. Broader repository coverage, matched
external-product runs, human/machine costs and complete capture-to-delivery
workflows remain required by the [competitive goal](../../docs/competitive-evaluation-goal-2026-11-15.md).
