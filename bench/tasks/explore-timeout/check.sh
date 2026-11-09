#!/usr/bin/env bash
# usage: check.sh <workdir> — exits 0 when the task is solved.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
work="$(cd "$1" && pwd)"
# check-lib.sh is linted on its own; the path here is computed.
# shellcheck disable=SC1091
. "$here/../../lib/check-lib.sh"
# The value lives in one place; the client's docstring, the docs and the
# visible tests (re-run pristine below) may be touched too, nothing else.
only_changed "$here" "$work" \
  relay/conf/defaults.py relay/transport/http_client.py docs/configuration.md README.md 'tests/*'

run_hidden_tests "$here" "$work"
