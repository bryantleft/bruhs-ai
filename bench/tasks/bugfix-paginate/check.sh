#!/usr/bin/env bash
# usage: check.sh <workdir> — exits 0 when the task is solved.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
# check-lib.sh is linted on its own; the path here is computed.
# shellcheck disable=SC1091
. "$here/../../lib/check-lib.sh"
run_hidden_tests "$here" "$(cd "$1" && pwd)"
