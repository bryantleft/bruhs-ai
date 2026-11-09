# Bench results

Measured in November 2026 with `scripts/task-bench.sh` and
`scripts/token-bench.sh` (see [README.md](README.md)) on one Mac. Claude
rows use Claude Haiku 5.5 (Claude Code 2.1.319) unless noted; Codex rows use
`gpt-6.1-sol` (codex-cli 0.166). Tokens are context tokens per finished run,
from each tool's own session logs. Per-task means swing 10–25% between
runs, so differences under ~5% are noise.

## Summary

- **Coding:** bruhs passes the same tasks as the CLIs it drives and spends
  11% fewer tokens than plain Claude Code (6% more than plain Codex).
- **Chat without a project:** a lean context saves 11–28% of tokens per
  question on Claude and 9% on Codex, at the same accuracy.
- **`/second-opinion`** (one provider reviews the other's answer) took
  Claude from 12/15 to 15/15 on the chat questions for 1.3× the tokens of
  plain Claude Code. On coding tasks it roughly doubled the tokens and did
  not change an outcome, so it stays a command you choose.

## Coding: 11 tasks × 2 runs

| tool | passed | tokens / run | vs plain CLI |
|---|---|---|---|
| Claude Code (plain) | 21/22 | 278,346 | — |
| **bruhs → Claude** | 21/22 | **248,674** | **−11%** |
| T3 Code → Claude | 22/22 | 424,120 | +52% |
| Codex CLI (plain) | 22/22 | 179,417 | — |
| bruhs → Codex | 22/22 | 191,017 | +6% |
| T3 Code → Codex | 22/22 | 200,730 | +12% |

The single misses are on tasks Haiku fails at random
(`concurrency-keyed-jobs`, `rootcause-paging`), so no tool is shown to be
smarter here.

## Chat: 5 trap questions × 3 runs

| tool | passed | tokens / question |
|---|---|---|
| Claude Code (plain) | 12/15 | 52,593 |
| bruhs → Claude (lean) | 12/15 | 38,611 |
| **bruhs → Claude + `/second-opinion` (Codex reviews)** | **15/15** | 68,077 |
| Codex CLI (plain) | 15/15 | 26,992 |
| **bruhs → Codex (lean)** | 15/15 | **24,498** |

With Claude Sonnet 4.6 instead of Haiku: plain Claude Code 13/15 at 41,119
tokens, bruhs → Claude 12/15 at 36,539 (−11%). Claude's misses are nearly
all one question (UK fuel economy, imperial vs US gallons). Across the
review runs the reviewer caught every wrong first answer (6 of 6) and
turned no right one wrong.

## Coding review on a task Haiku fails half the time

`rootcause-paging`, bruhs → Haiku with Codex reviewing, 6 runs:

| | passed | tokens / run |
|---|---|---|
| first answer | 4/6 | ~349k |
| + review and follow-up | 4/6 | 699k |

The reviewer found real but different edge cases; the failing rule went
unfixed both times.

## Per-request floor

One trivial turn in an empty git repo:

| tool | context tokens |
|---|---|
| Claude Code (user config) | 47.2k |
| Claude Code (plain, no plugins or MCP) | 32.1–33.7k |
| T3 Code → Claude | 47.3k |
| bruhs → Claude (lean) | 32.3k |
| Codex CLI (user config) | 24.0k |
| T3 Code → Codex | 21.4k |
| **bruhs → Codex (lean)** | **17.9k** |
| OpenCode (its free `big-pickle` model) | 17.9k |
| Aider (system prompt + repo map, no tool loop) | 2.0k |

Tokenizers differ across rows (OpenCode's model is neither Claude nor GPT,
and Aider's count is LiteLLM's estimate), so read them as ±10%. Aider runs
no agent loop, so its floor is not comparable with the others; a
same-model task run against it needs an API key and has not been done yet.
