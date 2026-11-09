#!/usr/bin/env bash
# Task bench: run the fixed tasks in bench/tasks through several agent
# harnesses and record pass/fail, tokens, peak context, turns and wall time.
# Coding tasks edit a repo; chat tasks (a `kind` file saying "chat") just
# answer, in the home directory with no project, and are graded from the
# captured final reply. See bench/README.md.
#
#   scripts/task-bench.sh                 run every task through every tool
#   scripts/task-bench.sh --self-check    prove each check.sh fails on the
#                                         untouched repo and passes on solution/
#   scripts/task-bench.sh --list          tasks and which tools are available
#   scripts/task-bench.sh --prepare TASK  make a fresh workdir (manual runs)
#   scripts/task-bench.sh --check TASK DIR  grade a workdir
#   scripts/task-bench.sh --rescore TS    re-parse target/bench/TS/ logs with the
#                                         current parsers and rewrite TS.md
#
# Env: BENCH_TASKS, BENCH_TOOLS (space or comma separated subsets),
# BENCH_RUNS (repeat each tool/task cell N times, default 1),
# BENCH_MODEL (claude + bruhs, default haiku), BENCH_CODEX_MODEL (codex; unset
# = codex's configured default), BENCH_BRUHS_MODEL, BENCH_BRUHS_CODEX_MODEL,
# BENCH_AIDER_MODEL, BENCH_OPENCODE_MODEL, BENCH_T3_BIN (default `t3` on PATH,
# else ~/.t3/bin/t3), BENCH_T3_PORT (its isolated server, default 3998),
# BENCH_CLAUDE_ARGS / BENCH_CODEX_ARGS
# (extra flags, word-split), BENCH_TIMEOUT (seconds per run,
# default 600), BENCH_WORK_ROOT (default ~/.cache/bruhs-bench), BRUHS_URL
# (default http://127.0.0.1:3001).
#
# Bash 3.2 compatible (macOS /bin/bash): no associative arrays or mapfile.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BENCH="$ROOT/bench"
TASKS_DIR="$BENCH/tasks"
LIB="$BENCH/lib"

ALL_TOOLS="claude codex bruhs bruhs-codex bruhs-review bruhs-codex-review aider opencode t3 t3-codex"
MODEL="${BENCH_MODEL:-haiku}"
CODEX_MODEL="${BENCH_CODEX_MODEL:-}"
BRUHS_URL="${BRUHS_URL:-http://127.0.0.1:3001}"
BRUHS_URL="${BRUHS_URL%/}"
BRUHS_MODEL="${BENCH_BRUHS_MODEL:-$MODEL}"
TIMEOUT="${BENCH_TIMEOUT:-600}"
WORK_ROOT="${BENCH_WORK_ROOT:-$HOME/.cache/bruhs-bench}"
RUNS="${BENCH_RUNS:-1}"
case "$RUNS" in '' | *[!0-9]* | 0) echo "task-bench: BENCH_RUNS must be a positive integer" >&2 && exit 1 ;; esac

T3_BIN="${BENCH_T3_BIN:-$(command -v t3 2>/dev/null || echo "$HOME/.t3/bin/t3")}"
T3_PORT="${BENCH_T3_PORT:-3998}"
T3_HOME=""
T3_PID=""

# Variables an agent host (T3 Code, Claude Code) sets for its own session.
# Started from such a terminal, every tool here would run as a child of that
# session: CLAUDE_CODE_ENTRYPOINT=t3-code alone gives Claude the host's
# Artifact tools and skills, 32k -> 46k tokens per turn (Claude Code 2.1.319).
# T3 sets its own entrypoint for the threads it starts, so `t3` still pays it.
for parent_var in CLAUDECODE CLAUDE_CODE_ENTRYPOINT CLAUDE_CODE_SESSION_ID \
  CLAUDE_CODE_CHILD_SESSION CLAUDE_CODE_SESSION_ATTENDED CLAUDE_CODE_MESSAGING_SOCKET \
  CLAUDE_CODE_MESSAGING_TOKEN CLAUDE_CODE_EXECPATH CLAUDE_PID CLAUDE_AGENT_SDK_VERSION \
  CLAUDE_EFFORT AI_AGENT T3CODE_THREAD_ID T3CODE_CLI_PATH T3CODE_DESKTOP_SHELL_DISCOVERY_PENDING; do
  unset "$parent_var"
done

say() { printf '%s\n' "$*" >&2; }
SKIPPED=""
skip() { # a tool that cannot run: tell the user now and list it in the report
  say "$*"
  SKIPPED="${SKIPPED:+$SKIPPED; }$*"
}
die() {
  say "task-bench: $*"
  exit 1
}

all_tasks() {
  local dir
  for dir in "$TASKS_DIR"/*/; do
    [ -f "$dir/prompt.md" ] && basename "$dir"
  done
}

selected_tasks() {
  local list="${BENCH_TASKS:-}" task
  if [ -z "$list" ]; then
    all_tasks
    return
  fi
  for task in ${list//,/ }; do
    [ -f "$TASKS_DIR/$task/prompt.md" ] || die "unknown task '$task' (have: $(all_tasks | tr '\n' ' '))"
    echo "$task"
  done
}

selected_tools() {
  local list="${BENCH_TOOLS:-$ALL_TOOLS}" tool
  for tool in ${list//,/ }; do
    case " $ALL_TOOLS " in
      *" $tool "*) echo "$tool" ;;
      *) die "unknown tool '$tool' (have: $ALL_TOOLS)" ;;
    esac
  done
}

task_kind() { # code (the default) or chat, from bench/tasks/<task>/kind
  local file="$TASKS_DIR/$1/kind"
  if [ -f "$file" ]; then tr -d '[:space:]' <"$file"; else echo code; fi
}

# Chat tasks need a harness that can answer without a project and hand back
# its final reply: the four headless ones. Review mode reviews code changes,
# aider/opencode are file editors here, and every T3 thread has a project.
tool_runs_kind() { # tool_runs_kind <tool> <kind>
  [ "$2" = code ] && return 0
  case "$1" in claude | codex | bruhs | bruhs-codex | bruhs-review | bruhs-codex-review) return 0 ;; esac
  return 1
}

explicitly_selected() { # was this tool named in BENCH_TOOLS?
  case " ${BENCH_TOOLS//,/ } " in *" $1 "*) return 0 ;; esac
  return 1
}

# Copy a task's repo/ into a fresh directory and commit it, so harnesses that
# want git (codex, aider) see a clean repo and nothing above it.
make_workdir() { # make_workdir <task> <dir>
  local task="$1" dir="$2"
  rm -rf "$dir"
  mkdir -p "$dir"
  cp -R "$TASKS_DIR/$task/repo/." "$dir/"
  (
    cd "$dir"
    git init -q
    git add -A
    git -c user.name=bench -c user.email=bench@localhost commit -qm "task start" --no-verify
  )
}

grade() { # grade <task> <dir> <log> -> exit status of check.sh
  "$TASKS_DIR/$1/check.sh" "$2" >"$3" 2>&1
}

codex_default_model() {
  local config="${CODEX_HOME:-$HOME/.codex}/config.toml"
  [ -f "$config" ] || return 0
  sed -n 's/^model *= *"\(.*\)"/\1/p' "$config" | head -1
}

bruhs_reachable() {
  local code
  code="$(curl -sS -m 3 -o /dev/null -w '%{http_code}' "$BRUHS_URL/api/agent/auth" 2>/dev/null || true)"
  [ -n "$code" ] && [ "$code" != "000" ]
}

# ---------------------------------------------------------------- modes

self_check() {
  local scratch task bad=0 base fixed
  scratch="$(mktemp -d "${TMPDIR:-/tmp}/task-bench-self.XXXXXX")"
  SELF_CHECK_SCRATCH="$scratch"
  trap 'rm -rf "$SELF_CHECK_SCRATCH"' EXIT
  printf '%-24s %-10s %-10s %s\n' task untouched solution wrong-samples
  for task in $(selected_tasks); do
    [ -d "$TASKS_DIR/$task/solution" ] || die "$task has no solution/"
    if [ "$(task_kind "$task")" = chat ]; then
      self_check_chat "$task" "$scratch" || bad=1
      continue
    fi
    make_workdir "$task" "$scratch/$task-base" >/dev/null
    if grade "$task" "$scratch/$task-base" "$scratch/$task-base.log"; then
      base="PASS(bad)"
      bad=1
      sed 's/^/    /' "$scratch/$task-base.log" >&2
    else
      base="fail(ok)"
    fi
    make_workdir "$task" "$scratch/$task-fixed" >/dev/null
    cp -R "$TASKS_DIR/$task/solution/." "$scratch/$task-fixed/"
    if grade "$task" "$scratch/$task-fixed" "$scratch/$task-fixed.log"; then
      fixed="pass(ok)"
    else
      fixed="FAIL(bad)"
      bad=1
      sed 's/^/    /' "$scratch/$task-fixed.log" >&2
    fi
    printf '%-24s %-10s %-10s\n' "$task" "$base" "$fixed"
  done
  if [ "$bad" = 0 ]; then
    echo "self-check ok: every check fails untouched (and on each wrong sample) and passes on its solution"
  else
    echo "self-check FAILED"
  fi
  return "$bad"
}

# A chat task: no reply and every solution/wrong-*.txt must fail, and
# solution/reply.txt must pass.
self_check_chat() { # self_check_chat <task> <scratch>
  local task="$1" dir="$2/$1" base fixed wrong=0 wrong_bad=0 sample bad=0
  mkdir -p "$dir"
  if grade "$task" "$dir" "$dir.base.log"; then
    base="PASS(bad)"
    bad=1
  else
    base="fail(ok)"
  fi
  cp "$TASKS_DIR/$task/solution/reply.txt" "$dir/reply.txt"
  if grade "$task" "$dir" "$dir.fixed.log"; then
    fixed="pass(ok)"
  else
    fixed="FAIL(bad)"
    bad=1
    sed 's/^/    /' "$dir.fixed.log" >&2
  fi
  for sample in "$TASKS_DIR/$task"/solution/wrong-*.txt; do
    [ -f "$sample" ] || continue
    wrong=$((wrong + 1))
    cp "$sample" "$dir/reply.txt"
    if grade "$task" "$dir" "$dir.wrong.log"; then
      wrong_bad=$((wrong_bad + 1))
      bad=1
      say "    $(basename "$sample") passed but should fail"
    fi
  done
  printf '%-24s %-10s %-10s %s\n' "$task" "$base" "$fixed" \
    "$([ "$wrong_bad" = 0 ] && echo "$wrong fail(ok)" || echo "$wrong_bad/$wrong PASS(bad)")"
  return "$bad"
}

list_mode() {
  local task tool
  echo "tasks:"
  for task in $(all_tasks); do
    printf '  %-24s %-5s %s\n' "$task" "$(task_kind "$task")" "$(head -1 "$TASKS_DIR/$task/prompt.md")"
  done
  echo "tools:"
  for tool in $ALL_TOOLS; do
    printf '  %-19s %s\n' "$tool" "$(tool_status "$tool")"
  done
}

tool_status() { # one-line availability for --list
  case "$1" in
    claude | codex | aider | opencode)
      if command -v "$1" >/dev/null 2>&1; then echo "available"; else echo "not on PATH"; fi
      ;;
    bruhs | bruhs-codex | bruhs-review | bruhs-codex-review)
      if bruhs_reachable; then echo "server at $BRUHS_URL"; else echo "server not reachable at $BRUHS_URL"; fi
      ;;
    t3 | t3-codex)
      if [ -x "$T3_BIN" ]; then echo "available ($T3_BIN, isolated server on :$T3_PORT)"; else echo "t3 CLI not found"; fi
      ;;
  esac
}

prepare_mode() { # --prepare <task> [dir]
  local task="${1:-}" dir="${2:-}"
  [ -n "$task" ] && [ -f "$TASKS_DIR/$task/prompt.md" ] || die "usage: --prepare <task> [dir]"
  dir="${dir:-$WORK_ROOT/manual/$task-$(date +%Y%m%d-%H%M%S)}"
  if [ "$(task_kind "$task")" = chat ]; then
    mkdir -p "$dir"
    echo "chat task: run it in a chat with no project (home directory), then save"
    echo "the agent's final reply as $dir/reply.txt"
  else
    make_workdir "$task" "$dir"
  fi
  echo "workdir: $dir"
  echo "prompt ($TASKS_DIR/$task/prompt.md):"
  echo "----"
  cat "$TASKS_DIR/$task/prompt.md"
  echo "----"
  echo "grade it with: scripts/task-bench.sh --check $task $dir"
}

check_mode() { # --check <task> <dir>
  local task="${1:-}" dir="${2:-}"
  [ -n "$task" ] && [ -d "$dir" ] || die "usage: --check <task> <dir>"
  if "$TASKS_DIR/$task/check.sh" "$dir"; then
    echo "PASS"
  else
    echo "FAIL"
    return 1
  fi
}

# ---------------------------------------------------------------- harnesses

# Each run_<tool> gets: <task> <workdir> <logbase> <prompt-file>, with KIND
# (code or chat), TOOL and RUN_N set. It writes <logbase>.log (the harness's
# machine-readable stream; format_for names its metrics.py format),
# <logbase>.err and <logbase>.time. Its exit status is the harness's.
#
# Chat tasks run in $HOME with no project: plain claude/codex get cwd $HOME
# (codex read-only), bruhs gets no project_path, which makes its Claude turns
# lean. <workdir> is then only where the captured reply goes.

run_cmd() { # run_cmd <workdir> <logbase> <cmd...>
  local work="$1" logbase="$2"
  shift 2
  python3 "$LIB/run.py" --timeout "$TIMEOUT" --time-file "$logbase.time" --cwd "$work" \
    --stdout "$logbase.log" --stderr "$logbase.err" -- "$@"
}

run_claude() {
  local extra=() cwd="$2"
  # shellcheck disable=SC2206 # word-splitting the flags is the point
  [ -n "${BENCH_CLAUDE_ARGS:-}" ] && extra=($BENCH_CLAUDE_ARGS)
  [ "$KIND" = chat ] && cwd="$HOME"
  # OAuth login, not an inherited API key (same rule as bruhs).
  run_cmd "$cwd" "$3" env -u ANTHROPIC_API_KEY claude -p --output-format stream-json --verbose \
    --model "$MODEL" --dangerously-skip-permissions ${extra[@]+"${extra[@]}"} -- "$(cat "$4")"
}

run_codex() {
  local model_args=()
  [ -n "$CODEX_MODEL" ] && model_args=(-m "$CODEX_MODEL")
  # shellcheck disable=SC2206 # word-splitting the flags is the point
  [ -n "${BENCH_CODEX_ARGS:-}" ] && model_args+=($BENCH_CODEX_ARGS)
  if [ "$KIND" = chat ]; then
    run_cmd "$HOME" "$3" codex exec --json --sandbox read-only \
      --skip-git-repo-check ${model_args[@]+"${model_args[@]}"} -C "$HOME" "$(cat "$4")"
    return
  fi
  run_cmd "$2" "$3" codex exec --json --dangerously-bypass-approvals-and-sandbox \
    --skip-git-repo-check ${model_args[@]+"${model_args[@]}"} -C "$2" "$(cat "$4")"
}

# bruhs: the app's own agent path, POST /api/agent on bruhs-server, which
# streams the turn as SSE. A native client (no Origin header) on loopback
# passes the server's local-access guard; the project path must be under
# $HOME, which is why the default work root is ~/.cache/bruhs-bench.
bench_thread() { # bench_thread <task>: one thread per (run, tool, task, repeat)
  printf 'bench-%s-%s-%s-%s' "$RUN_ID" "$TOOL" "$1" "$RUN_N"
}

bruhs_post() { # bruhs_post <logbase> <body.json> <cwd>: writes <logbase>.{log,http,err,time}
  local logbase="$1" body="$2" status=0
  python3 "$LIB/run.py" --timeout "$TIMEOUT" --time-file "$logbase.time" --cwd "$3" \
    --stdout "$logbase.http" --stderr "$logbase.err" -- \
    curl -sS -N -o "$logbase.log" -w '%{http_code}' -H 'Content-Type: application/json' \
    --data-binary "@$body" "$BRUHS_URL/api/agent" || status=$?
  if [ "$(cat "$logbase.http" 2>/dev/null)" != "200" ]; then
    say "  bruhs: HTTP $(cat "$logbase.http" 2>/dev/null): $(head -c 300 "$logbase.log" 2>/dev/null)"
    [ "$status" = 0 ] && status=1
  fi
  return "$status"
}

# run_bruhs_with <provider> <model> <task> <workdir> <logbase> <prompt-file>
# A coding task sends the workdir as project_path with bypassPermissions. A
# chat task sends no project_path (a home-directory chat: lean Claude
# context); Claude keeps bypassPermissions like plain `claude`, Codex gets
# the default mode, which bruhs runs read-only like plain `codex` above.
run_bruhs_with() {
  local provider="$1" model="$2" task="$3" work="$4" logbase="$5" prompt="$6"
  local project="$work" permission=bypassPermissions
  if [ "$KIND" = chat ]; then
    project=""
    [ "$provider" = codex ] && permission=default
  fi
  python3 "$LIB/review.py" turn-body "$logbase.body.json" "$prompt" "$project" \
    "$(bench_thread "$task")" "$model" "$permission" ""
  bruhs_post "$logbase" "$logbase.body.json" "$work"
}

run_bruhs() {
  run_bruhs_with claude "$BRUHS_MODEL" "$@"
}

run_bruhs_codex() {
  run_bruhs_with codex "$BRUHS_CODEX_MODEL" "$@"
}

# Review mode: the bruhs turn as above, then POST /api/agent/second-opinion
# (the other provider reviews read-only), then one follow-up turn on the
# same thread and provider session carrying the review, as bruhs's
# /second-opinion composer draft does. Writes <logbase>.log (turn 1),
# .t1.pass (turn 1 graded on a copy of the workdir),
# .review.body.json/.review.json/.review.time, .t2.log (turn 2); .time is
# the sum. When turn 1 or the review fails, the reason goes to .review.skip
# and the run is graded as turn 1 left it.
run_review_with() { # run_review_with <provider> <model> <task> <workdir> <logbase> <prompt-file>
  local provider="$1" model="$2" task="$3" work="$4" logbase="$5" prompt="$6" status=0 session
  # A chat task has no project: the review and turn 2 run as a home-dir
  # chat, like turn 1 (run_bruhs_with), and turn 1 is graded on its reply.
  local project="$work" permission=bypassPermissions
  if [ "$KIND" = chat ]; then
    project=""
    [ "$provider" = codex ] && permission=default
  fi
  run_bruhs_with "$@" || status=$?
  mv "$logbase.time" "$logbase.t1.time" 2>/dev/null || true
  session="$(python3 "$LIB/review.py" session "$logbase.log")"
  python3 "$LIB/metrics.py" --reply "bruhs-$provider" "$logbase.log" >"$logbase.t1.reply.txt"
  [ "$KIND" = chat ] && cp "$logbase.t1.reply.txt" "$work/reply.txt"
  grade_copy "$task" "$work" "$logbase.t1"
  if [ "$status" != 0 ] || [ -z "$session" ] || [ ! -s "$logbase.t1.reply.txt" ]; then
    echo "turn 1 failed (exit $status) or left no session/reply" >"$logbase.review.skip"
    sum_times "$logbase.time" "$logbase.t1.time"
    return "$status"
  fi
  python3 "$LIB/review.py" review-body "$logbase.review.body.json" "$prompt" \
    "$logbase.t1.reply.txt" "$project" "$provider"
  python3 "$LIB/run.py" --timeout "$TIMEOUT" --time-file "$logbase.review.time" --cwd "$work" \
    --stdout "$logbase.review.http" --stderr "$logbase.review.err" -- \
    curl -sS -o "$logbase.review.json" -w '%{http_code}' -H 'Content-Type: application/json' \
    --data-binary "@$logbase.review.body.json" "$BRUHS_URL/api/agent/second-opinion" || true
  say "  review: HTTP $(cat "$logbase.review.http" 2>/dev/null), $(cat "$logbase.review.time" 2>/dev/null)s"
  if ! python3 "$LIB/review.py" followup "$logbase.review.json" "$logbase.t2.prompt.md" >"$logbase.review.skip"; then
    say "  review: $(cat "$logbase.review.skip")"
    sum_times "$logbase.time" "$logbase.t1.time" "$logbase.review.time"
    return 0
  fi
  rm -f "$logbase.review.skip"
  python3 "$LIB/review.py" turn-body "$logbase.t2.body.json" "$logbase.t2.prompt.md" "$project" \
    "$(bench_thread "$task")" "$model" "$permission" "$session"
  bruhs_post "$logbase.t2" "$logbase.t2.body.json" "$work" || status=$?
  sum_times "$logbase.time" "$logbase.t1.time" "$logbase.review.time" "$logbase.t2.time"
  return "$status"
}

# Grade a copy of <workdir> into <logbase>.check / .pass (1 or 0), so the
# hidden tests never land where a later turn could read them.
grade_copy() { # grade_copy <task> <workdir> <logbase>
  local copy
  copy="$(mktemp -d "${TMPDIR:-/tmp}/task-bench-grade.XXXXXX")"
  cp -R "$2/." "$copy/"
  if grade "$1" "$copy" "$3.check"; then echo 1 >"$3.pass"; else echo 0 >"$3.pass"; fi
  rm -rf "$copy"
}

sum_times() { # sum_times <out> <time-file...>: missing files count 0
  local out="$1"
  shift
  cat "$@" 2>/dev/null | awk '{ s += $1 } END { printf "%.1f\n", s }' >"$out"
}

run_bruhs_review() {
  run_review_with claude "$BRUHS_MODEL" "$@"
}

run_bruhs_codex_review() {
  run_review_with codex "$BRUHS_CODEX_MODEL" "$@"
}

# T3 Code: a server of its own (`t3 serve --base-dir` under the run's work
# root, port BENCH_T3_PORT), so runs never touch the user's T3 data, and one
# thread per run through the CLI (`t3 thread start --wait --json`), each in
# a project made from its workdir. Tokens come from the provider's session
# logs in the workdir (<logbase>.cwd names it; metrics.py t3-*).
t3_start() {
  [ -n "$T3_HOME" ] && return 0
  T3_HOME="$WORK_ROOT/$RUN_ID/t3home"
  mkdir -p "$T3_HOME"
  "$T3_BIN" serve --base-dir "$T3_HOME" --port "$T3_PORT" --no-browser >"$T3_HOME/serve.log" 2>&1 &
  T3_PID=$!
  trap t3_stop EXIT
  local tries=0
  until curl -s -m 2 -o /dev/null "http://127.0.0.1:$T3_PORT/"; do
    tries=$((tries + 1))
    if [ "$tries" -ge 60 ] || ! kill -0 "$T3_PID" 2>/dev/null; then
      say "T3 server did not start on :$T3_PORT (log: $T3_HOME/serve.log)"
      return 1
    fi
    sleep 1
  done
}

t3_stop() {
  [ -n "$T3_PID" ] && kill "$T3_PID" 2>/dev/null || true
}

run_t3_with() { # run_t3_with <provider instance> <model> <task> <workdir> <logbase> <prompt-file>
  local work="$4" logbase="$5"
  echo "$work" >"$logbase.cwd"
  if ! "$T3_BIN" project add --base-dir "$T3_HOME" "$work" >"$logbase.project" 2>&1; then
    say "  t3: project add failed: $(head -c 300 "$logbase.project")"
    return 1
  fi
  run_cmd "$work" "$logbase" "$T3_BIN" thread start --base-dir "$T3_HOME" --provider "$1" \
    --model "$2" --wait --json "$work" "$(cat "$6")"
}

run_t3() {
  run_t3_with claudeAgent "$MODEL" "$@"
}

run_t3_codex() {
  run_t3_with codex "$BRUHS_CODEX_MODEL" "$@"
}

run_aider() {
  local model_args=()
  [ -n "${BENCH_AIDER_MODEL:-}" ] && model_args=(--model "$BENCH_AIDER_MODEL")
  # aider prints its token report on stdout; keep that as the log.
  run_cmd "$2" "$3" aider --yes-always --no-auto-commits --no-check-update --no-pretty \
    ${model_args[@]+"${model_args[@]}"} --message-file "$4"
}

run_opencode() {
  local model_args=()
  [ -n "${BENCH_OPENCODE_MODEL:-}" ] && model_args=(--model "$BENCH_OPENCODE_MODEL")
  run_cmd "$2" "$3" opencode run --format json ${model_args[@]+"${model_args[@]}"} "$(cat "$4")"
}

# Whether <tool> can run here; prints why not (or nothing, for tools that
# skip silently) and returns 1 when it cannot.
tool_ready() {
  case "$1" in
    claude | codex)
      command -v "$1" >/dev/null 2>&1 || {
        skip "skip $1: '$1' is not on PATH"
        return 1
      }
      ;;
    aider | opencode)
      command -v "$1" >/dev/null 2>&1 || {
        explicitly_selected "$1" && skip "skip $1: '$1' is not on PATH"
        return 1
      }
      ;;
    bruhs | bruhs-codex | bruhs-review | bruhs-codex-review)
      bruhs_reachable || {
        skip "skip $1: bruhs-server is not reachable at $BRUHS_URL (start it with 'cargo run -p bruhs-server' or set BRUHS_URL)"
        return 1
      }
      case "$1" in
        bruhs-codex | bruhs-codex-review)
          if [ -z "$BRUHS_CODEX_MODEL" ]; then
            skip "skip $1: no codex model (set BENCH_BRUHS_CODEX_MODEL or BENCH_CODEX_MODEL)"
            return 1
          fi
          ;;
      esac
      ;;
    t3 | t3-codex)
      [ -x "$T3_BIN" ] || {
        skip "skip $1: the t3 CLI is not installed (set BENCH_T3_BIN)"
        return 1
      }
      if [ "$1" = t3-codex ] && [ -z "$BRUHS_CODEX_MODEL" ]; then
        skip "skip $1: no codex model (set BENCH_BRUHS_CODEX_MODEL or BENCH_CODEX_MODEL)"
        return 1
      fi
      t3_start || {
        skip "skip $1: the T3 server did not start"
        return 1
      }
      ;;
  esac
}

bench() {
  local ts out_dir results tasks tools tool task kind skipped_kinds
  ts="$(date +%Y%m%d-%H%M%S)"
  RUN_ID="$ts"
  out_dir="$ROOT/target/bench/$ts"
  mkdir -p "$out_dir"
  results="$out_dir/results.tsv"
  printf 'tool\ttask\tkind\trun\tpass\ttokens\tpeak_ctx\tturns\ttool_calls\twall_s\texit\tnote\n' >"$results"
  tasks="$(selected_tasks)"
  tools="$(selected_tools)"
  BRUHS_CODEX_MODEL="${BENCH_BRUHS_CODEX_MODEL:-${CODEX_MODEL:-$(codex_default_model)}}"

  for tool in $tools; do
    tool_ready "$tool" || continue
    skipped_kinds=""
    for task in $tasks; do
      kind="$(task_kind "$task")"
      if ! tool_runs_kind "$tool" "$kind"; then
        skipped_kinds="${skipped_kinds:+$skipped_kinds }$task"
        continue
      fi
      RUN_N=1
      while [ "$RUN_N" -le "$RUNS" ]; do
        run_one "$tool" "$task" "$kind" "$ts" "$out_dir" "$results"
        RUN_N=$((RUN_N + 1))
      done
    done
    [ -z "$skipped_kinds" ] || skip "$tool does not run chat tasks ($skipped_kinds)"
  done

  local report="$ROOT/target/bench/$ts.md"
  {
    echo "run $ts, tasks: $(echo "$tasks" | tr '\n' ' ')"
    echo "runs per tool/task: $RUNS"
    echo "claude model: $MODEL; codex model: ${CODEX_MODEL:-configured default ($(codex_default_model))}; bruhs model: $BRUHS_MODEL; bruhs-codex model: ${BRUHS_CODEX_MODEL:-none}"
    echo "per-run timeout ${TIMEOUT}s (per step in review mode); logs and results.tsv in target/bench/$ts/; workdirs in $WORK_ROOT/$ts/"
    [ -z "$SKIPPED" ] || echo "not run: $SKIPPED"
  } >"$out_dir/meta.txt"
  render_report "$out_dir" >"$report"
  cat "$report"
  say ""
  say "report: $report"
}

run_one() { # run_one <tool> <task> <kind> <ts> <out_dir> <results>; RUN_N is the repeat
  local tool="$1" task="$2" kind="$3" ts="$4" out_dir="$5" results="$6"
  local work logbase status pass metrics note wall format
  work="$WORK_ROOT/$ts/$tool/$task-$RUN_N"
  logbase="$out_dir/$tool-$task-$RUN_N"
  format="$(format_for "$tool")"
  if [ "$kind" = chat ]; then
    rm -rf "$work"
    mkdir -p "$work"
    say "→ $tool / $task #$RUN_N  (chat in \$HOME; reply to $work/reply.txt)"
  else
    make_workdir "$task" "$work"
    say "→ $tool / $task #$RUN_N  (workdir $work)"
  fi
  KIND="$kind"
  TOOL="$tool"
  status=0
  "run_${tool//-/_}" "$task" "$work" "$logbase" "$TASKS_DIR/$task/prompt.md" || status=$?
  python3 "$LIB/metrics.py" --reply "$format" "$logbase.log" >"$logbase.reply.txt" || true
  [ "$kind" = chat ] && cp "$logbase.reply.txt" "$work/reply.txt"
  pass=0
  if grade "$task" "$work" "$logbase.check"; then pass=1; fi
  metrics="$(python3 "$LIB/metrics.py" "$format" "$logbase.log")"
  wall="$(cat "$logbase.time" 2>/dev/null || echo "-")"
  note="$(status_note "$status" "$(printf '%s' "$metrics" | cut -f5)")"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$tool" "$task" "$kind" "$RUN_N" "$pass" \
    "$(printf '%s' "$metrics" | cut -f1-4)" "$wall" "$status" "$note" >>"$results"
  say "  $([ "$pass" = 1 ] && echo PASS || echo FAIL)  tokens=$(printf '%s' "$metrics" | cut -f1)  wall=${wall}s"
}

status_note() { # status_note <exit> <metrics note>
  if [ "$1" = 124 ]; then
    echo "timeout after ${TIMEOUT}s; $2"
  elif [ "$1" != 0 ]; then
    echo "exit $1; $2"
  else
    echo "$2"
  fi
}

render_report() { # render_report <run dir>: markdown from results.tsv + meta.txt
  local dir="$1" meta=()
  if [ -f "$dir/meta.txt" ]; then
    while IFS= read -r line; do meta+=("$line"); done <"$dir/meta.txt"
  fi
  python3 "$LIB/report.py" "$dir/results.tsv" ${meta[@]+"${meta[@]}"}
}

format_for() { # metrics.py format of a tool's log
  case "$1" in
    bruhs) echo bruhs-claude ;;
    bruhs-review) echo bruhs-review-claude ;;
    bruhs-codex-review) echo bruhs-review-codex ;;
    t3) echo t3-claude ;;
    *) echo "$1" ;;
  esac
}

rescore_mode() { # --rescore <timestamp>
  local ts="${1:-}" dir results tmp tool task kind run pass wall status note metrics stem
  dir="$ROOT/target/bench/$ts"
  results="$dir/results.tsv"
  [ -n "$ts" ] && [ -f "$results" ] || die "usage: --rescore <timestamp> (a directory under target/bench/)"
  tmp="$results.new"
  # Runs before BENCH_RUNS had no kind/run columns and logs named <tool>-<task>.
  if head -1 "$results" | grep -q "$(printf '\tkind\t')"; then
    head -1 "$results" >"$tmp"
    # shellcheck disable=SC2034 # the _ fields are recomputed
    tail -n +2 "$results" | while IFS=$'\t' read -r tool task kind run pass _ _ _ _ wall status _; do
      metrics="$(python3 "$LIB/metrics.py" "$(format_for "$tool")" "$dir/$tool-$task-$run.log")"
      note="$(status_note "$status" "$(printf '%s' "$metrics" | cut -f5)")"
      printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$tool" "$task" "$kind" "$run" "$pass" \
        "$(printf '%s' "$metrics" | cut -f1-4)" "$wall" "$status" "$note" >>"$tmp"
    done
  else
    printf 'tool\ttask\tkind\trun\tpass\ttokens\tpeak_ctx\tturns\ttool_calls\twall_s\texit\tnote\n' >"$tmp"
    # shellcheck disable=SC2034 # the _ fields are recomputed
    tail -n +2 "$results" | while IFS=$'\t' read -r tool task pass _ _ _ _ wall status _; do
      stem="$dir/$tool-$task"
      metrics="$(python3 "$LIB/metrics.py" "$(format_for "$tool")" "$stem.log")"
      note="$(status_note "$status" "$(printf '%s' "$metrics" | cut -f5)")"
      printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$tool" "$task" code 1 "$pass" \
        "$(printf '%s' "$metrics" | cut -f1-4)" "$wall" "$status" "$note" >>"$tmp"
    done
  fi
  mv "$tmp" "$results"
  echo "rescored $(date +%Y-%m-%d) with the current parsers" >>"$dir/meta.txt"
  render_report "$dir" >"$ROOT/target/bench/$ts.md"
  cat "$ROOT/target/bench/$ts.md"
}

usage() { # the header comment, up to the first non-comment line
  awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0"
}

case "${1:-}" in
  --self-check) self_check ;;
  --list) list_mode ;;
  --prepare)
    shift
    prepare_mode "$@"
    ;;
  --check)
    shift
    check_mode "$@"
    ;;
  --rescore)
    shift
    rescore_mode "$@"
    ;;
  -h | --help) usage ;;
  "") bench ;;
  *)
    usage >&2
    exit 2
    ;;
esac
