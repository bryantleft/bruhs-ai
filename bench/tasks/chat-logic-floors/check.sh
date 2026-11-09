#!/usr/bin/env bash
# usage: check.sh <workdir> — a chat task: grades <workdir>/reply.txt, the
# agent's final reply as captured by the bench runner.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
python3 "$here/../../lib/answer.py" "$1/reply.txt" text Eli 4
