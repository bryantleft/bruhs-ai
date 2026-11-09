# Task bench

A small, repeatable benchmark that runs the same tasks through several agent
harnesses and records, per (tool, task, repeat): whether the task's own
hidden check passes, tokens spent, peak context, model requests, tool calls
and wall time. Coding tasks edit a repo and are graded by hidden tests; chat
tasks only answer a question and are graded from the captured final reply.
It checks whether bruhs spends fewer tokens per finished task **and**
passes more tasks than the tools it wraps or competes with. Latest numbers:
[RESULTS.md](RESULTS.md).

```sh
scripts/task-bench.sh --self-check        # no agent: every check fails untouched, passes on solution/
scripts/task-bench.sh --list              # tasks, and which tools are available here
BENCH_TOOLS=claude,codex BENCH_TASKS=bugfix-paginate,explore-timeout scripts/task-bench.sh
BENCH_RUNS=3 BENCH_TOOLS=claude,bruhs scripts/task-bench.sh   # every cell 3 times: mean, min/max
scripts/task-bench.sh                     # everything available (spends real tokens)
scripts/task-bench.sh --rescore <timestamp>  # re-parse a past run's logs after a parser fix
```

The markdown report goes to stdout and to `target/bench/<timestamp>.md`; raw
logs (`<tool>-<task>-<run>.*`), check output, captured replies
(`.reply.txt`) and `results.tsv` go to `target/bench/<timestamp>/`. Agent
workdirs are kept under `~/.cache/bruhs-bench/<timestamp>/<tool>/<task>-<run>/`
so you can look at what each agent did (`git diff` in the workdir — each one
starts as a one-commit repo; a chat task's directory only holds `reply.txt`).

## Tasks

Each `bench/tasks/<name>/` holds:

| file | |
|---|---|
| `prompt.md` | the exact instruction given to the agent |
| `repo/` | the starting files, copied fresh for every run (Python 3 stdlib only) |
| `hidden/` | tests the agent never sees; copied in only after it finishes |
| `check.sh <workdir>` | exits 0 when the task is solved |
| `solution/` | a reference solution, overlaid on `repo/` by `--self-check` |

`check.sh` runs the hidden tests together with pristine copies of the repo's
visible tests (so weakening a visible test doesn't help) from
`<workdir>/_bench_hidden/`.

| task | kind | what it measures |
|---|---|---|
| `bugfix-paginate` | bug fix | off-by-one in `page_count`; trivial for any agent, so mostly fixed per-run overhead |
| `feature-inventory-csv` | feature over 2–3 files | `Item.total_value` + `csv_report` + a `--csv` CLI flag |
| `refactor-shipping` | refactor, tests stay green | collapse three copy-pasted `if/elif` country chains into `RATES`/`rate_for`; checked for identical behaviour against the original and for each country code appearing once |
| `explore-timeout` | find-and-change in a 33-file repo | the HTTP default timeout lives in `relay/conf/defaults.py` among other 30-second settings; any other file changing fails (docstring/docs edits allowed) |
| `longfile-fee-policy` | read a long file | a 900-line / ~70 KB `POLICY.md`; the grace-status late fee is wrong, and the cap is in an amendment near the end |
| `multibug-ledger` | several bugs, 3 files | five independent bugs in `money.py` (fraction digits, sign of `-0.50`), `accounts.py` (inclusive `as_of`, same-day order) and `report.py` (negative cents); the prompt names two symptoms, one visible test fails, the docstrings are the spec |
| `spec-business-days` | implement from a spec | five stubs against `SPEC.md`: business-day stepping from weekend days, holidays, four roll conventions (modified following/preceding cross month ends), `add_months` clamping and end-of-month rule incl. leap years, schedules computed from the start date (no drift) |
| `invariant-rename` | feature that must respect a hidden invariant | `Store.rename_prefix`; the rules live only in comments in other modules: every change goes through `_commit` (the query cache keys on the version), one log entry per call (the replica applies entries as transactions), prefixes are normalised, and a batch applies in order (deletes before puts when prefixes overlap) |
| `rootcause-paging` | bug whose symptom points at another file | keyset paging in `orders/`: the reported symptom (orders missing when several share a second) shows in `api.list_orders`, but the causes are elsewhere: `store.sort_key` has no id tie-break (and must break ties highest id first), `timeutil.to_micros` uses `timetuple()` and so ignores UTC offsets; plus `next_cursor` must be `None` when nothing follows a full page, and `decode_cursor` must reject every string `encode_cursor` could not produce (`[]`, `[1, 2, 3]`, `[7]`, …); checked against a reference listing with mixed offsets and while orders are added and removed between pages |
| `spec-invoice-tax` | implement from a long spec | `compute_invoice` against an 11-section `SPEC.md`: half away from zero for lines vs half to even for group tax, tax per rate group (numerically equal rates merge), largest-remainder invoice discount over positive lines only, tax-inclusive prices, per-line tax split rounded toward negative infinity, 0/2/3-decimal currencies, `-0.00`, every value of the wrong type a `TypeError` |
| `concurrency-keyed-jobs` | asyncio bug fix under cancellation | `KeyedSerializer` (one job at a time per key) built as a chain of futures: a dropped waiting job lets the next one start early, a cancelled caller cancels a shared future, and `drain` can be cancelled into the jobs; the obvious rewrite (a per-key `Lock` dropped when `not lock.locked()`) races on hand-over, which the hidden tests catch |

The first five are easy (every harness passed all of them in the first
runs); the other six exist to separate tools on intelligence and were meant
to make a small model (Haiku) fail some of the time. Measured with plain
`claude --model haiku` (Haiku 5.5), they mostly do not: `multibug-ledger`,
`spec-business-days` and `invariant-rename` pass every run, as do
`spec-invoice-tax` (12/12 over runs `20261108-115557`, `-121125`,
`-123733`, across its versions) and `concurrency-keyed-jobs` (8/8,
`-121125`, `-123733`). Only `rootcause-paging` lands in range: 3/8 with
the current hidden tests (runs `-115557` and `-121125`, re-graded; Haiku
misses the cursor-shape rule, accepting `[]` or any list of ints), and 4/4
in `-123733`, a run whose `encode_cursor` docstring spelled out that rule
(since reverted). Haiku implements whatever a spec states, races and DST
included; it slips where it has to derive the valid domain itself.

Adding a task: create the five pieces above, make `check.sh` source
`bench/lib/check-lib.sh` and call `run_hidden_tests` (and `only_changed` if
the task forbids collateral edits), then run `--self-check`.

### Chat tasks

A task directory with a `kind` file containing `chat` is a chat task: no
repo, nothing to edit, the agent just answers. Layout:

| file | |
|---|---|
| `kind` | `chat` |
| `prompt.md` | the question; it says not to touch files and to end with one `ANSWER: …` line |
| `check.sh <dir>` | grades `<dir>/reply.txt` with `bench/lib/answer.py` (the last `ANSWER:` line; tolerant of markdown and units) |
| `solution/reply.txt` | a reply that must pass; `solution/wrong-*.txt` are plausible wrong replies that must fail |

The runner captures the final reply itself (Claude `result.result`, Codex's
last `agent_message`, bruhs's `result` event or last text) into
`<run>.reply.txt` and the reply directory; the agent is never asked to write
a file, because these chats run in the home directory. They measure bruhs's
**lean chat**: a request with no `project_path` runs in `$HOME` with no
Claude settings files (no plugins, skills or settings-enabled MCP servers).
To compare like with like, the plain tools run in `$HOME` too: `claude` with
cwd `$HOME` and its usual flags, `codex exec -C $HOME --sandbox read-only`;
`bruhs` sends no `project_path` with `bypassPermissions` (as plain `claude`
skips permissions), `bruhs-codex` sends no `project_path` with the default
permission mode, which bruhs runs read-only for Codex.

`claude`, `codex`, `bruhs`, `bruhs-codex` and the two review tools run chat
tasks; `aider` and `opencode` skip them (listed under "not run"). In review
mode a chat's review and follow-up also run with no project, and turn 1 is
graded on its own reply, so `turn 1 alone` shows whether the review fixed a
wrong answer.

| task | what it measures |
|---|---|
| `chat-due-date` | 25 business days past a Friday with two holidays, then +90 calendar days, and the weekday (2027-06-07, Monday) |
| `chat-logic-floors` | 5 people × floors × pets, 11 clues, unique solution (every clue needed; checked by brute force) |
| `chat-fuel-units` | UK mpg to L/100 km and US mpg: the twist is the imperial gallon (5.40, 43.5) |
| `chat-business-days` | 30 business days across the year end with three holidays; skipping them gives 2027-01-11 (2027-01-14, Thursday) |
| `chat-friday-13` | count Friday-the-13ths over five years and name the last (9, 2030-12-13) |

## Harnesses

Every run starts in a fresh copy of `repo/`, headless, with no approval
prompts. `BENCH_TIMEOUT` (default 600 s) kills the whole process tree.

| tool | how it runs | tokens / context source |
|---|---|---|
| `claude` | `env -u ANTHROPIC_API_KEY claude -p --output-format stream-json --verbose --model $BENCH_MODEL --dangerously-skip-permissions` (OAuth login) | `result.usage`; peak from `assistant` messages deduped by `message.id` |
| `codex` | `codex exec --json --dangerously-bypass-approvals-and-sandbox --skip-git-repo-check [-m $BENCH_CODEX_MODEL]` | last `turn.completed.usage`; peak and request count from the session rollout `~/.codex/sessions/**/rollout-*-<thread_id>.jsonl` (`token_count` events) |
| `bruhs` | `POST $BRUHS_URL/api/agent` on bruhs-server — the same route the web app uses — with `permission_mode: bypassPermissions`, `fresh_session: true`, model `$BENCH_BRUHS_MODEL` (default `$BENCH_MODEL`); reads the SSE stream | the stream's `result.usage`; peak and request count from Claude Code's own session transcript `~/.claude/projects/*/<session_id>.jsonl` |
| `bruhs-codex` | same route with a Codex model (`$BENCH_BRUHS_CODEX_MODEL`, else `$BENCH_CODEX_MODEL`, else `model` from `~/.codex/config.toml`) | last `usage` event; peak from the codex rollout |
| `aider` | `aider --yes-always --no-auto-commits --no-check-update --no-pretty --message-file prompt.md [--model $BENCH_AIDER_MODEL]` — only when on PATH | `Tokens: N sent, M received` lines (aider rounds them) |
| `opencode` | `opencode run --format json [--model $BENCH_OPENCODE_MODEL]` — only when on PATH | `step_finish` token parts |
| `bruhs-review` | `bruhs`, then review mode (below) with Codex reviewing | turn 1 + turn 2 from the streams; the review's own tokens when its session log can be found (see below) |
| `bruhs-codex-review` | `bruhs-codex`, then review mode with Claude reviewing | same |
| `t3` | `t3 thread start --provider claudeAgent --model $BENCH_MODEL --wait --json <workdir> <prompt>` on an isolated T3 server (below) | every top-level Claude transcript in `~/.claude/projects/<workdir>/` (assistant messages deduped by id; matched `result.usage` exactly on two plain-`claude` runs) |
| `t3-codex` | same with `--provider codex` and the bruhs-codex model | every Codex rollout whose `session_meta.cwd` is the workdir |

`aider` and `opencode` are skipped silently when missing (with a message only
if you named them in `BENCH_TOOLS`). Their parsers were written against the
tools' documented output and have **not** been exercised here (neither is
installed on the dev Mac), so treat their first numbers with suspicion.

### bruhs

Start the server first (`cargo run -p bruhs-server`, or `scripts/dev.sh`).
`BRUHS_URL` defaults to `http://127.0.0.1:3001`; if nothing answers there the
bruhs tools are skipped with a message. The bench talks to the server as a
native client: no `Origin` header, loopback `Host`, which the local-access
guard accepts (see `docs/local-server-access.md`). The server only runs turns
in directories under `$HOME`, which is why workdirs live in
`~/.cache/bruhs-bench` (set `BENCH_WORK_ROOT` to move them, still under
`$HOME`). Workdirs are deliberately outside this repo: Claude Code loads every
`CLAUDE.md` above its cwd and Codex reads `AGENTS.md` up to the git root, so
a workdir inside the bruhs-ai checkout would bill bruhs-ai's instructions to
every run.

Each bench turn uses a new thread id `bench-<timestamp>-<tool>-<task>-<run>`.

### Review mode (`bruhs-review`, `bruhs-codex-review`)

Coding tasks only. What a user gets from bruhs's `/second-opinion`, scripted:

1. the bruhs turn exactly as `bruhs` / `bruhs-codex` runs it;
2. `POST $BRUHS_URL/api/agent/second-opinion` with `{"project_path": <workdir>,
   "thread_provider": "claude"|"codex", "task": <prompt>, "answer": <turn 1's
   final reply>, "focus": ""}`: the other provider reviews read-only;
3. one follow-up turn on the same thread id, resuming turn 1's provider
   session (`session_id` from its `init`/`result` event), with the prompt
   bruhs's composer drafts: "A second reviewer (<Reviewer>) looked at your
   last answer: … Check each point (against the code, where there is code).
   Fix what is right, and say briefly why you disagree with the rest. If your
   answer changes, state the final answer again in full." (the text of
   `bruhs_models::logic::handoff::SECOND_OPINION_FOLLOWUP_ASK`). When the
   review opens with `VERDICT: holds up`, bruhs drafts no follow-up and
   neither does the bench: the run ends after the review, graded as turn 1
   left it (note: "no turn 2");
4. grade. Turn 1 is also graded on a copy of the workdir (so the hidden
   tests never reach turn 2); the note says `turn 1 alone: PASS|FAIL`, which
   is what tells you whether the review changed the outcome.

`tokens` is turn 1 + turn 2 + the review when it can be counted. The
endpoint returns no usage, so the bench looks for the reviewer's own session
log in the workdir: a Claude reviewer's transcript under
`~/.claude/projects/<workdir>/` (counted like a transcript: output
undercounted), a Codex reviewer's rollout whose `session_meta.cwd` is the
workdir. The Codex reviewer runs `--ephemeral` and writes no rollout, so for
`bruhs-review` the note says "tokens = turns only". The review's wall time
is always in the note and in `wall_s`. Peak and request count come from the
resumed session's log once, since it covers both turns. If turn 1 fails or
the review errors, turn 2 is skipped, the note says why
(`<run>.review.skip`), and the run is graded as turn 1 left it.
`BENCH_TIMEOUT` applies to each of the three steps.

### T3 Code

`t3` and `t3-codex` start a T3 server of their own once per bench run
(`t3 serve --base-dir <work root>/<run>/t3home --port $BENCH_T3_PORT`,
default 3998), so the runs never touch your T3 projects or threads. Each run
adds its workdir as a project and starts one thread through the CLI. Code
tasks only: every T3 thread belongs to a project. A timeout stops the CLI
client, not the thread; the server is stopped when the bench exits.

### Parent sessions

The bench unsets the variables an agent host sets for its own session
(`CLAUDECODE`, `CLAUDE_CODE_ENTRYPOINT`, the messaging socket, …) before it
runs anything. Started from a T3 or Claude Code terminal, every tool would
otherwise run as that host's child, and `CLAUDE_CODE_ENTRYPOINT=t3-code`
alone adds T3's Artifact tools to Claude (32k -> 46k tokens per turn). T3
sets that entrypoint itself for its own threads, so `t3` still pays it, as
it does in the app. Runs before 2026-11-08 evening were made from a T3
terminal: their `claude` and `bruhs` rows include the extra ~14k per
request, equally for both.

## Reading the numbers

- **pass** — the task's `check.sh` exit status after the agent stopped. The
  only correctness signal; an agent saying "done" counts for nothing.
- **tokens** — everything billed for the run: uncached input + cache reads +
  cache writes + output, over every model request. For Codex,
  `input_tokens` already includes `cached_input_tokens`, so spent is
  `input + output` (cached is not added again). Cache reads are cheap in
  money but are still context the model processed; this column is volume,
  not cost. Divide by model price yourself if you want dollars. Claude
  numbers come from `result.usage` for both `claude` and `bruhs` (the bruhs
  stream only forwards that); Claude Code's `result.modelUsage` also counts
  small side requests and reads slightly higher (under 1 % in the runs so
  far). When a bruhs stream ends without its final usage event, Codex
  tokens fall back to the session rollout's running total and the note says
  so.
- **peak_ctx** — the largest single prompt (input incl. cached) any request
  in the run sent. Tells you how close a harness gets to the context window
  and how much it drags along per step.
- **turns** — model requests: distinct assistant message ids for Claude and
  bruhs (Claude's `result.num_turns` counts something else and runs 2–4
  higher), distinct `token_count` reports in the rollout for Codex.
- **tool_calls** — tool invocations (Claude `tool_use` blocks; Codex
  command/file-change/MCP/web-search items).
- **wall_s** — wall-clock seconds for the harness process, including CLI
  start-up.
- **Per tool** — one row per tool and task kind (code, chat): runs passed
  out of runs made (tasks × `BENCH_RUNS`), mean / min / max tokens per run,
  and `tokens / passed run`, total tokens over all its runs divided by the
  runs it passed: the headline "tokens per finished task". Failed runs still
  count toward the numerator. With `BENCH_RUNS` > 1 a "Per tool and task"
  table adds pass count and mean / min / max per cell; every run keeps its
  own row in "Runs".

Caveats:

- One run per cell is noisy. Agents are nondeterministic; use
  `BENCH_RUNS=3` (or more) before reading much into a 10–20 % difference.
- Every Claude-based run (plain `claude` and `bruhs`) starts with your
  user-level Claude Code setup: `~/.claude/CLAUDE.md`, plugins, skills,
  hooks and MCP servers. On a heavily customised machine that is tens of
  thousands of tokens of fixed cost per run and dominates the small tasks.
  That is the real cost on this machine, and it is the same for `claude`
  and `bruhs`. To measure the harnesses bare, pass extra flags with
  `BENCH_CLAUDE_ARGS` (e.g. `--strict-mcp-config --setting-sources project`)
  and `BENCH_CODEX_ARGS`; bruhs-server has no such switch.
- `BENCH_MODEL=haiku` (the default) keeps runs cheap. Codex uses whatever
  `~/.codex/config.toml` configures unless `BENCH_CODEX_MODEL` is set, so
  Claude and Codex rows are different models; compare like with like
  (`claude` vs `bruhs`, `codex` vs `bruhs-codex`) before comparing across
  providers.

## Environment

| variable | default | |
|---|---|---|
| `BENCH_TASKS` | all | space/comma list of task names |
| `BENCH_TOOLS` | `claude codex bruhs bruhs-codex bruhs-review bruhs-codex-review aider opencode t3 t3-codex` | space/comma list |
| `BENCH_RUNS` | `1` | repeats per (tool, task) |
| `BENCH_MODEL` | `haiku` | claude `--model` |
| `BENCH_CODEX_MODEL` | codex config | codex `-m` |
| `BENCH_BRUHS_MODEL` | `$BENCH_MODEL` | model id sent to bruhs |
| `BENCH_BRUHS_CODEX_MODEL` | `$BENCH_CODEX_MODEL` or codex config | |
| `BENCH_AIDER_MODEL`, `BENCH_OPENCODE_MODEL` | tool default | |
| `BENCH_T3_BIN`, `BENCH_T3_PORT` | `t3` on PATH, else `~/.t3/bin/t3`; `3998` | T3's CLI and its isolated server |
| `BENCH_CLAUDE_ARGS`, `BENCH_CODEX_ARGS` | none | extra flags, word-split |
| `BENCH_TIMEOUT` | `600` | seconds per run (per step in review mode) |
| `BENCH_WORK_ROOT` | `~/.cache/bruhs-bench` | must be under `$HOME` for bruhs |
| `BRUHS_URL` | `http://127.0.0.1:3001` | bruhs-server |

Files: `scripts/task-bench.sh` (driver, bash 3.2-compatible),
`bench/lib/run.py` (timeout + timing), `bench/lib/metrics.py` (log parsers
and reply capture), `bench/lib/review.py` (bruhs request bodies, review-mode
glue), `bench/lib/answer.py` (chat grading), `bench/lib/report.py`
(markdown), `bench/lib/check-lib.sh` (shared check helpers).
