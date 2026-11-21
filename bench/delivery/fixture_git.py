"""Read-only Git proxy for owned delivery source-guard fixtures."""
from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path


def main() -> int:
    args = sys.argv[1:]
    command = args
    settings_allowed = True
    while command[:1] == ["-c"] and len(command) >= 3:
        settings_allowed &= command[1] in {"core.fsmonitor=false", "core.quotePath=true", "diff.autoRefreshIndex=false"}
        command = command[2:]
    allowed = settings_allowed and (
        command == ["rev-parse", "HEAD"]
        or command == ["symbolic-ref", "--short", "HEAD"]
        or command == ["config", "--get", "remote.origin.url"]
        or command == ["diff", "--binary", "HEAD"]
        or command == ["diff", "--binary", "--full-index", "--no-ext-diff", "--no-textconv", "HEAD", "--"]
        or command[:3] == ["ls-files", "--error-unmatch", "--"]
    )
    with Path(os.environ["BRUHS_GUARD_GIT_LOG"]).open("a") as handle:
        handle.write(json.dumps({"args": args, "command": command, "allowed": allowed, "cwd": os.getcwd()}) + "\n")
    if not allowed:
        print("Owned fixture blocked a mutating or unexpected Git command before execution.", file=sys.stderr)
        return 97
    return subprocess.run([os.environ["BRUHS_GUARD_REAL_GIT"], *args], check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main())
