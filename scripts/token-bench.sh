#!/usr/bin/env bash
# Harness token probe: how many context tokens one trivial turn costs under
# each launch configuration bruhs could use, plus a resumed turn to confirm
# the prompt cache survives a new process. Spends real tokens (a few hundred
# thousand cached reads on the chosen model), so it is not part of the gate.
#
#   scripts/token-bench.sh                 # Claude (haiku) + Codex
#   BENCH_CLAUDE_MODEL=sonnet scripts/token-bench.sh
#   BENCH_SKIP_CODEX=1 scripts/token-bench.sh
#   BENCH_SKIP_RIVALS=1 scripts/token-bench.sh   # skip OpenCode and Aider
#
# Runs in a throwaway git repo so the project's CLAUDE.md does not count.
# Results go to stdout; compare them with bench/RESULTS.md.
set -euo pipefail

model=${BENCH_CLAUDE_MODEL:-haiku}
probe="Reply with just: ok"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$work"
git init -q && echo hi >a.txt && git add . && git -c user.name=b -c user.email=b@b commit -qm init

# Context tokens of a Claude run: fresh input + cache reads + cache writes.
claude_ctx() {
  env -u ANTHROPIC_API_KEY claude -p --output-format json --model "$model" "$@" -- "$probe" |
    jq -r '"\(.usage.input_tokens + .usage.cache_creation_input_tokens + .usage.cache_read_input_tokens)\t\(.session_id)"'
}

no_mcp=(--strict-mcp-config '--mcp-config={"mcpServers":{}}')

printf '%-44s %s\n' "configuration" "context tokens"
row() { printf '%-44s %s\n' "$1" "$2"; }

read -r today sid < <(claude_ctx)
row "claude: bruhs chat today (full user config)" "$today"
row "claude: + no MCP servers" "$(claude_ctx "${no_mcp[@]}" | cut -f1)"
row "claude: + no user settings/plugins" \
  "$(claude_ctx "${no_mcp[@]}" --setting-sources=project,local | cut -f1)"

resumed=$(env -u ANTHROPIC_API_KEY claude -p --output-format json --model "$model" \
  --resume="$sid" -- "$probe" | jq -r '"\(.usage.cache_creation_input_tokens) written / \(.usage.cache_read_input_tokens) read"')
row "claude: resumed in a new process (cache)" "$resumed"

if [ "${BENCH_SKIP_CODEX:-0}" != 1 ] && command -v codex >/dev/null; then
  # turn.completed reports the thread's running total; input includes cached.
  codex_ctx=$(codex exec --json --skip-git-repo-check -s read-only -- "$probe" 2>/dev/null |
    jq -r 'select(.type=="turn.completed").usage | "\(.input_tokens) (\(.cached_input_tokens) cached)"')
  row "codex: exec, user config" "$codex_ctx"
  # bruhs lean (project-less chats): the user's config minus plugins.
  codex_lean=$(codex exec --json --skip-git-repo-check -s read-only -c features.plugins=false \
    -- "$probe" 2>/dev/null |
    jq -r 'select(.type=="turn.completed").usage | "\(.input_tokens) (\(.cached_input_tokens) cached)"')
  row "codex: exec, no plugins (bruhs lean)" "$codex_lean"
fi

# Rivals, when installed. Neither can use the Claude Code or Codex login, so
# OpenCode runs a free model of its own (its tokenizer, not Claude's), and
# Aider reports its prompt size locally (`/tokens`, Claude tokenizer
# estimate) without calling a model. A `--` before OpenCode's prompt hangs.
if [ "${BENCH_SKIP_RIVALS:-0}" != 1 ] && command -v opencode >/dev/null; then
  oc_model=${BENCH_OPENCODE_MODEL:-opencode/big-pickle}
  oc=$(opencode run --format json --model "$oc_model" "$probe" </dev/null 2>/dev/null |
    jq -r 'select(.type=="step_finish").part.tokens | .input + .cache.read + .cache.write')
  row "opencode: user config ($oc_model)" "$oc"
fi
if [ "${BENCH_SKIP_RIVALS:-0}" != 1 ] && command -v aider >/dev/null; then
  # A full model id: an unknown alias picks another edit format and prompt.
  aider_tokens=$(aider --model "${BENCH_AIDER_MODEL:-anthropic/claude-haiku-4-5}" \
    --yes-always --no-check-update \
    --no-pretty --no-show-model-warnings --no-gitignore --message /tokens </dev/null 2>/dev/null |
    awk '/tokens total/ { gsub(",", "", $3); print $3 }')
  row "aider: system prompt + repo map (no call)" "$aider_tokens"
fi
