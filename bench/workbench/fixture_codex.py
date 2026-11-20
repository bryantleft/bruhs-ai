"""Owned Codex protocol fixture. Never contacts a model or executes prompt code."""

from __future__ import annotations

import hashlib
import json
import os
import re
import signal
import sys
from pathlib import Path


def emit(value: dict[str, object]) -> None:
    print(json.dumps(value), flush=True)


def main() -> int:
    arguments = sys.argv[1:]
    workspace = Path(next(arg.removeprefix("--cd=") for arg in arguments if arg.startswith("--cd="))).resolve()
    home = Path(os.environ["HOME"]).resolve()
    if not workspace.is_relative_to(home):
        raise ValueError("Fixture may only write inside its synthetic home.")
    prompt = sys.stdin.read()
    cases = re.findall(r"FIXTURE_CASE=(success|failed_exit|truncated|blocked|stalled_exit|review_findings)", prompt)
    if not cases:
        raise ValueError("Fixture prompt must name an explicit test case.")
    case = cases[-1]
    requested_session = arguments[arguments.index("--") + 1] if "resume" in arguments else None
    session = requested_session or "fixture-" + hashlib.sha256(str(workspace).encode()).hexdigest()
    invocation = {"case": case, "workspace": str(workspace), "pid": os.getpid(), "session": session,
                  "sandbox": next((arg.removeprefix("--sandbox=") for arg in arguments if arg.startswith("--sandbox=")), None),
                  "resume": requested_session,
                  "effort": next((json.loads(arg.split("=", 2)[2]) for arg in reversed(arguments)
                                  if arg.startswith("--config=model_reasoning_effort=")), None)}
    with Path(os.environ["BRUHS_FIXTURE_LOG"]).open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(invocation) + "\n")
    if case == "blocked":
        release = workspace / "fixture-release"
        os.mkfifo(release)
    emit({"type": "thread.started", "thread_id": session})
    emit({"type": "item.completed", "item": {"type": "agent_message", "id": "progress", "text": "Inspecting fixture"}})
    if case == "blocked":
        with release.open(encoding="utf-8") as handle:
            handle.read()
    if case != "review_findings":
        (workspace / "outcome.txt").write_text(session, encoding="utf-8")
    answer = ("FAIL: synthetic regression check reports an unresolved defect. Review the evidence before accepting."
              if case == "review_findings" else "Verified final answer")
    emit({"type": "item.completed", "item": {"type": "agent_message", "id": "answer", "text": answer}})
    if case != "truncated":
        emit({"type": "turn.completed", "usage": {"input_tokens": 20, "cached_input_tokens": 7, "output_tokens": 3}})
    if case == "failed_exit":
        print("Intentional fixture failure after turn.completed", file=sys.stderr)
        return 7
    if case == "stalled_exit":
        os.close(sys.stdout.fileno())
        while True:
            signal.pause()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
