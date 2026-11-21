"""Synthetic Codex protocol and harmless owned process tree; never invokes a model."""
from __future__ import annotations

import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import tempfile
import time


def atomic_write(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}-", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as handle:
            handle.write(data)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


def record(row: dict) -> None:
    path = Path(os.environ["BRUHS_FIXTURE_LOG"])
    with path.with_suffix(".lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        previous = path.read_bytes() if path.exists() else b""
        atomic_write(path, previous + (json.dumps(row) + "\n").encode())


def emit(value: dict) -> None:
    print(json.dumps(value), flush=True)


def stall() -> None:
    while True:
        signal.pause()


def main() -> int:
    arguments = sys.argv[1:]
    if arguments[:1] == ["--owned-budget-child"]:
        record({"role": "child", "synthetic": True, "pid": os.getpid(), "ppid": os.getppid(),
                "session": arguments[1], "workspace": str(Path.cwd()), "started_at_ns": time.monotonic_ns()})
        atomic_write(Path.cwd() / f"budget-child-{arguments[1]}.txt", b"SYNTHETIC harmless child started.\n")
        stall()
        return 0
    workspace = Path(next(arg.removeprefix("--cd=") for arg in arguments if arg.startswith("--cd="))).resolve()
    home = Path(os.environ["HOME"]).resolve()
    if workspace == home or not workspace.is_relative_to(home):
        raise ValueError("Synthetic provider requires a private project inside its owned HOME.")
    prompt = sys.stdin.read()
    # Choose the last marker: resumed/forked prompts may retain old stall markers.
    cases = re.findall(r"BUDGET_FIXTURE_CASE=(stall|hold|success|resume|flood)", prompt)
    if not cases:
        raise ValueError("Synthetic provider requires an explicit BUDGET_FIXTURE_CASE marker.")
    case = cases[-1]
    resumed = arguments[arguments.index("--") + 1] if "resume" in arguments else None
    session = resumed or "synthetic-budget-" + hashlib.sha256(str(workspace).encode()).hexdigest()[:20]
    child = None
    if case in {"stall", "hold", "flood"}:
        child = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), "--owned-budget-child", session],
                                 cwd=workspace, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    record({"role": "provider", "synthetic": True, "case": case, "pid": os.getpid(),
            "child_pid": child.pid if child else None, "workspace": str(workspace), "session": session,
            "resume": resumed, "arguments": arguments, "started_at_ns": time.monotonic_ns(),
            "sandbox": next((arg.removeprefix("--sandbox=") for arg in arguments if arg.startswith("--sandbox=")), None),
            "effort": next((json.loads(arg.split("=", 2)[2]) for arg in reversed(arguments)
                            if arg.startswith("--config=model_reasoning_effort=")), None)})
    atomic_write(workspace / "budget-partial.txt", f"SYNTHETIC partial file retained for {session}.\n".encode())
    emit({"type": "thread.started", "thread_id": session})
    partial = "SYNTHETIC fixture partial output: harmless private project edit is saved. "
    emit({"type": "item.completed", "item": {"type": "agent_message", "id": "synthetic-partial", "text": partial}})
    if case == "flood":
        for number in range(8192):
            emit({"type": "item.completed", "item": {"type": "agent_message", "id": f"synthetic-flood-{number}",
                  "text": "SYNTHETIC backpressure sample " + "x" * 2048}})
    if case in {"stall", "hold", "flood"}:
        stall()
    atomic_write(workspace / "budget-completed.txt", f"SYNTHETIC completed file for {session}.\n".encode())
    emit({"type": "item.completed", "item": {"type": "agent_message", "id": "synthetic-final", "text": "SYNTHETIC explicit completion."}})
    emit({"type": "turn.completed", "usage": {"input_tokens": 23, "cached_input_tokens": 5, "output_tokens": 7}})
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
