"""Run one harness command with a wall-clock timeout and time it.

usage: run.py --timeout SECS --time-file PATH --cwd DIR --stdout PATH
              --stderr PATH [--stdin PATH] -- CMD [ARGS...]

The command runs in its own process group so a timeout kills the whole
tree (agent CLIs spawn shells and helpers). Exit status is the command's,
or 124 on timeout. Elapsed seconds are written to --time-file.
"""

from __future__ import annotations

import argparse
import math
import os
import signal
import subprocess
import sys
import time


def stop_process_group(proc: subprocess.Popen) -> None:
    try:
        os.killpg(proc.pid, signal.SIGTERM)
    except ProcessLookupError:
        proc.wait()
        return
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        pass
    finally:
        # The group can outlive its leader when a helper ignores SIGTERM.
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    proc.wait()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--timeout", type=float, required=True)
    parser.add_argument("--time-file", required=True)
    parser.add_argument("--cwd", required=True)
    parser.add_argument("--stdout", required=True)
    parser.add_argument("--stderr", required=True)
    parser.add_argument("--stdin")
    parser.add_argument("cmd", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    cmd = args.cmd[1:] if args.cmd[:1] == ["--"] else args.cmd
    if not cmd:
        parser.error("no command")
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("--timeout must be positive and finite")

    stdin = open(args.stdin, "rb") if args.stdin else subprocess.DEVNULL
    started = time.monotonic()
    status = 0
    with open(args.stdout, "wb") as out, open(args.stderr, "wb") as err:
        proc = subprocess.Popen(
            cmd, cwd=args.cwd, stdin=stdin, stdout=out, stderr=err, start_new_session=True
        )
        try:
            status = proc.wait(timeout=args.timeout)
        except subprocess.TimeoutExpired:
            stop_process_group(proc)
            status = 124
        except KeyboardInterrupt:
            stop_process_group(proc)
            raise
    elapsed = time.monotonic() - started
    with open(args.time_file, "w", encoding="utf-8") as handle:
        handle.write(f"{elapsed:.1f}\n")
    if stdin is not subprocess.DEVNULL:
        stdin.close()
    return status


if __name__ == "__main__":
    sys.exit(main())
