"""Grade frozen Python fixtures under Seatbelt with complete test accounting."""

from __future__ import annotations

import json
import math
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from bench.lib.run import stop_process_group
from bench.lib.test_receipts import (
    FrozenTests,
    check_receipt,
    reserve_outputs,
    save_result,
)
from bench.lib.test_receipts import (
    result_path as result_path,
)


def _profile_path(path: Path) -> str:
    return json.dumps(str(path.resolve()))


def grade(fixture: Path, work: Path, output: Path, timeout: float) -> int:
    """Return zero only after all frozen checks pass and the process settles.

    The raw log and host-validated `.result.json` sidecar are retained. Submitted
    code and tests share an interpreter, so this is not adversarial attestation.
    """
    if not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("Grading requires a positive finite observation timeout.")
    if output.exists() or output.is_symlink():
        raise ValueError(
            f"Grading artifact already exists: {output}. Choose a fresh output path to retain every attempt."
        )
    reserve_outputs(output)
    tests = FrozenTests.read(fixture)
    with tempfile.TemporaryDirectory(prefix="bruhs-grade-") as temporary:
        directory = Path(temporary)
        target = directory / "work"
        shutil.copytree(
            work,
            target,
            symlinks=True,
            ignore=shutil.ignore_patterns(
                ".git", "__pycache__", "tests", "_bench_hidden"
            ),
        )
        if any(path.is_symlink() for path in target.rglob("*")):
            raise ValueError("Generated workspace contains a symlink; grading refused.")
        tests.install(target)
        runner = directory / "test_runner.py"
        shutil.copy2(Path(__file__).resolve().parents[1] / "lib/test_runner.py", runner)
        channel_path = directory / "receipt.json"
        # The receipt channel is the only additional write exception. Network,
        # user-data reads and writes outside this temporary tree stay restricted.
        profile = "\n".join(
            [
                "(version 1)",
                "(allow default)",
                "(deny network*)",
                f"(deny file-write* (require-all (require-not (subpath {_profile_path(target)})) "
                f"(require-not (literal {_profile_path(channel_path)}))))",
                f"(deny file-read* (require-all (subpath {_profile_path(Path.home())}) "
                f"(require-not (subpath {_profile_path(Path(sys.base_prefix))})) "
                f"(require-not (subpath {_profile_path(target)})) "
                f"(require-not (literal {_profile_path(runner)}))))",
            ]
        )
        status = None
        raw = b""
        with output.open("xb") as log:
            if tests.expected:
                with channel_path.open("x+b") as channel:
                    process = subprocess.Popen(
                        [
                            "/usr/bin/sandbox-exec",
                            "-p",
                            profile,
                            sys.executable,
                            "-I",
                            "-S",
                            str(runner),
                            str(channel.fileno()),
                        ],
                        cwd=target,
                        stdin=subprocess.DEVNULL,
                        stdout=log,
                        stderr=subprocess.STDOUT,
                        env={
                            "PATH": "/usr/bin:/bin",
                            "HOME": str(target),
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
            for error in result.errors:
                log.write(("\nGRADING REJECTED: " + error + "\n").encode())
        save_result(output, raw, result)
        return 0 if result.passed else (124 if status == 124 else 1)
