"""Run the task bench's local coding checker with complete test receipts.

This preserves the task bench's local execution boundary; it adds no OS sandbox.
Campaigns supply their declared per-step timeout and retained output prefix.
Standalone checks can omit them and report their result on standard output.
"""

from __future__ import annotations

import argparse
import math
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from bench.lib.run import stop_process_group  # noqa: E402
from bench.lib.test_receipts import (  # noqa: E402
    FrozenTests,
    check_receipt,
    reserve_outputs,
    save_result,
    verify_result,
)


def run(fixture: Path, work: Path, output: Path | None, timeout: float | None) -> int:
    if timeout is not None and (not math.isfinite(timeout) or timeout <= 0):
        raise ValueError("Grading requires a positive finite observation timeout.")
    if output is not None:
        reserve_outputs(output)
    tests = FrozenTests.read(fixture)
    tests.install(work)
    status = None
    raw = b""
    if tests.expected:
        with tempfile.TemporaryFile() as channel:
            process = subprocess.Popen(
                [
                    sys.executable,
                    "-I",
                    "-S",
                    str(Path(__file__).with_name("test_runner.py")),
                    str(channel.fileno()),
                ],
                cwd=work,
                stdin=subprocess.DEVNULL,
                env={
                    "PATH": "/usr/bin:/bin",
                    "HOME": str(work),
                    "PYTHONDONTWRITEBYTECODE": "1",
                },
                pass_fds=(channel.fileno(),),
                start_new_session=True,
            )
            try:
                status = process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                stop_process_group(process)
                status = 124
            except BaseException:
                stop_process_group(process)
                raise
            channel.seek(0)
            raw = channel.read()
    result = check_receipt(status, raw, tests.expected)
    if output is not None:
        save_result(output, raw, result)
    for error in result.errors:
        print("GRADING REJECTED: " + error, file=sys.stderr)
    print(
        f"Independent checks: {'PASS' if result.passed else 'FAIL'} ({len(tests.expected)} required)"
    )
    return 0 if result.passed else (124 if status == 124 else 1)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixture", type=Path)
    parser.add_argument("work", type=Path, nargs="?")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--timeout", type=float)
    parser.add_argument("--verify-result", action="store_true")
    args = parser.parse_args()
    try:
        if args.verify_result:
            if args.output is None or args.work is not None:
                parser.error("--verify-result requires --output and no work directory")
            return 0 if verify_result(args.fixture, args.output).passed else 1
        if args.work is None:
            parser.error("a work directory is required to run independent checks")
        return run(
            args.fixture.resolve(), args.work.resolve(), args.output, args.timeout
        )
    except (OSError, ValueError) as error:
        print("GRADING REJECTED: " + str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
