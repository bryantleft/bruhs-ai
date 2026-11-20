"""Exercise real Workbench HTTP, provider-process and restart boundaries in isolation."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import http.client
import json
import math
import os
from pathlib import Path
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import time
import traceback
from collections.abc import Callable


ROOT = Path(__file__).resolve().parents[2]
ACTIVE = {"ready", "running", "waiting_approval", "waiting_child"}
SOURCE_INPUTS = [Path(__file__), Path(__file__).with_name("fixture_codex.py"),
                 ROOT / "bruhs-core/src/agent.rs", ROOT / "bruhs-core/src/coordination.rs",
                 ROOT / "codex-sdk-rs/src/query.rs", ROOT / "codex-sdk-rs/src/lib.rs",
                 ROOT / "claude-sdk-rs/src/process.rs",
                 ROOT / "Cargo.lock"]


def digest(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


class Campaign:
    def __init__(self, server: Path, output: Path, timeout: float) -> None:
        self.output = output.resolve()
        self.output.mkdir(parents=True, exist_ok=False)
        self.timeout = timeout
        self.home = self.output / "home"
        self.home.mkdir()
        self.project = self.home / "project"
        self.project.mkdir()
        self.workbench = self.output / "workbench"
        self.calls = self.output / "calls.jsonl"
        self.worker = self.output / "fixture_codex.py"
        shutil.copy2(Path(__file__).with_name("fixture_codex.py"), self.worker)
        self.server = self.output / "server-under-test"
        shutil.copy2(server, self.server)
        self.env = {
            "HOME": str(self.home), "XDG_DATA_HOME": str(self.home / "data"),
            "XDG_CONFIG_HOME": str(self.home / "config"),
            "CODEX_HOME": str(self.home / "codex"), "CLAUDE_CONFIG_DIR": str(self.home / "claude"),
            "BRUHS_WORKBENCH_DIR": str(self.workbench),
            "BRUHS_MEETING_DB": str(self.home / "meetings.sqlite3"),
            "BRUHS_CAPTURE_URL": "http://127.0.0.1:0",
            "BRUHS_CAPTURE_TOKEN_FILE": str(self.home / "missing-capture-token"),
            "BRUHS_WEB_DIST": str(self.home / "no-web-bundle"),
            "BRUHS_FIXTURE_LOG": str(self.calls),
            "RUST_LOG": "bruhs_server=info", "RUST_BACKTRACE": "1",
            "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
        }
        binary_dir = self.home / "bin"
        binary_dir.mkdir()
        codex = binary_dir / "codex"
        codex.write_text(f'#!/bin/sh\nexec {shlex.quote(sys.executable)} {shlex.quote(str(self.worker))} "$@"\n', encoding="utf-8")
        codex.chmod(0o700)
        self.env["PATH"] = f"{binary_dir}:/usr/bin:/bin"
        git = shutil.which("git")
        if git is None:
            raise RuntimeError("Workbench fixture requires Git.")
        (self.project / "sentinel.txt").write_text("Original project must stay unchanged.\n", encoding="utf-8")
        for args in (["init", "-b", "main"], ["add", "sentinel.txt"],
                     ["-c", "user.name=fixture", "-c", "user.email=fixture@localhost", "commit", "-m", "fixture baseline"]):
            subprocess.run([git, "-C", str(self.project), *args], env=self.env, check=True, capture_output=True)
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            self.port = reservation.getsockname()[1]
        self.env["BRUHS_SERVER_PORT"] = str(self.port)
        self.process: subprocess.Popen | None = None
        self.server_log = (self.output / "server.log").open("ab")

    def start(self) -> None:
        self.server_log.seek(0, os.SEEK_END)
        log_offset = self.server_log.tell()
        self.process = subprocess.Popen([str(self.server)], env=self.env, cwd=self.home,
                                        stdout=self.server_log, stderr=subprocess.STDOUT, start_new_session=True)

        def ready() -> dict | None:
            if self.process is None or self.process.poll() is not None:
                raise RuntimeError("Owned fixture server exited before readiness; inspect server.log.")
            with (self.output / "server.log").open("rb") as handle:
                handle.seek(log_offset)
                log = handle.read().decode(errors="replace")
            if f"bruhs-server listening on http://127.0.0.1:{self.port}" not in log:
                return None
            return self.api("GET", "/api/coordination")
        self.wait(ready, "server readiness", connecting=True)

    def stop(self) -> None:
        if self.process is not None and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=self.timeout)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
                raise RuntimeError("Owned server did not stop within the campaign budget.")

    def api(self, method: str, path: str, body: dict[str, object] | None = None) -> dict:
        if self.process is None or self.process.poll() is not None:
            raise RuntimeError("Owned fixture server exited; inspect server.log.")
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=self.timeout)
        try:
            connection.request(method, path, json.dumps(body) if body is not None else None,
                               {"Content-Type": "application/json"})
            response = connection.getresponse()
            value = response.read().decode()
            if response.status != 200:
                raise RuntimeError(f"{method} {path} returned {response.status}: {value}")
            return json.loads(value)
        finally:
            connection.close()

    def wait(self, observe: Callable, label: str, connecting: bool = False):
        deadline = time.monotonic() + self.timeout
        while time.monotonic() < deadline:
            try:
                result = observe()
                if result:
                    return result
            except (ConnectionRefusedError, ConnectionResetError):
                if not connecting:
                    raise
            # Sampling is for functional observation, not a latency benchmark.
            time.sleep(0.05)
        raise TimeoutError(f"{label} exceeded the campaign's {self.timeout}s observation budget.")

    def task(self, task_id: str) -> dict:
        return next(task for task in self.api("GET", "/api/coordination")["tasks"] if task["id"] == task_id)

    def command(self, **command: object) -> dict:
        return self.api("POST", "/api/coordination/commands", command)

    def create(self, case: str) -> dict:
        title = f"fixture-{case}-{time.monotonic_ns()}"
        snapshot = self.command(action="create", parent_id=None, task={
            "title": title, "project_path": str(self.project), "model": "codex/fixture-model",
            "prompt": f"FIXTURE_CASE={case}", "permission_mode": "acceptEdits",
            "isolation": {"kind": "worktree", "base_branch": "main"},
        })
        return next(task for task in snapshot["tasks"] if task["title"] == title)

    def settled(self, task_id: str) -> dict:
        return self.wait(lambda: (task if (task := self.task(task_id))["state"] not in ACTIVE else None), "task settlement")

    def invocations(self) -> list[dict]:
        return [json.loads(line) for line in self.calls.read_text(encoding="utf-8").splitlines()] if self.calls.exists() else []

    def owned_worker_alive(self, pid: int) -> bool:
        result = subprocess.run(["/bin/ps", "-ww", "-p", str(pid), "-o", "stat=", "-o", "command="],
                                capture_output=True, text=True, check=False).stdout.strip()
        return bool(result and not result.startswith("Z") and str(self.worker) in result)

    def running(self, task_id: str) -> dict:
        def initialized() -> dict | None:
            task = self.task(task_id)
            if task["state"] not in ACTIVE:
                raise RuntimeError(f"Task ended before provider initialization: {task}")
            return task if task["state"] == "running" and task["session_id"] else None
        return self.wait(initialized, "provider initialization")

    def completed(self) -> None:
        task = self.settled(self.create("success")["id"])
        assert task["state"] == "completed", task
        assert task["messages"][-1]["text"] == "Verified final answer", task["messages"]
        assert Path(task["working_path"], "outcome.txt").read_text() == task["session_id"]

    def failed_exit(self) -> None:
        task = self.settled(self.create("failed_exit")["id"])
        assert task["state"] == "failed" and "7" in task["error"], task

    def truncated(self) -> None:
        task = self.settled(self.create("truncated")["id"])
        assert task["state"] == "failed" and "completion" in task["error"], task

    def stalled_exit(self) -> None:
        task = self.settled(self.create("stalled_exit")["id"])
        assert task["state"] == "failed" and "after closing stdout" in task["error"], task

    def delegation(self) -> None:
        parent = self.settled(self.create("success")["id"])
        snapshot = self.command(action="fork", id=parent["id"], title="Isolated child",
                                prompt="FIXTURE_CASE=success", relationship="delegate", model="codex/fixture-model",
                                isolation={"kind": "worktree", "base_branch": "main"})
        child = next(task for task in snapshot["tasks"] if task["parent_id"] == parent["id"])
        child = self.settled(child["id"])
        assert parent["state"] == child["state"] == "completed", (parent, child)
        assert parent["working_path"] != child["working_path"]
        assert parent["session_id"] != child["session_id"]
        for task in (parent, child):
            assert Path(task["working_path"], "outcome.txt").read_text() == task["session_id"]
            calls = [call for call in self.invocations() if call["workspace"] == task["working_path"]]
            assert len(calls) == 1 and calls[0]["resume"] is None, calls
        assert not (self.project / "outcome.txt").exists(), "Provider wrote into original checkout."

    def cancellation(self) -> None:
        task = self.running(self.create("blocked")["id"])
        call = next(call for call in self.invocations() if call["workspace"] == task["working_path"])
        self.command(action="cancel", id=task["id"])
        assert self.task(task["id"])["state"] == "cancelled"
        self.wait(lambda: not self.owned_worker_alive(call["pid"]), "cancelled provider exit")

    def recovery(self) -> None:
        completed = [task for task in self.api("GET", "/api/coordination")["tasks"] if task["state"] == "completed"]
        artifacts = {task["id"]: Path(task["working_path"], "outcome.txt").read_bytes() for task in completed}
        task = self.running(self.create("blocked")["id"])
        call = next(call for call in self.invocations() if call["workspace"] == task["working_path"])
        before = len(self.invocations())
        self.stop()
        self.wait(lambda: not self.owned_worker_alive(call["pid"]), "provider exit before restart")
        self.start()
        for saved in completed:
            restored = self.task(saved["id"])
            assert restored["state"] == "completed", restored
            assert restored["session_id"] == saved["session_id"]
            assert restored["messages"] == saved["messages"]
            assert Path(restored["working_path"], "outcome.txt").read_bytes() == artifacts[saved["id"]]
        restored = self.task(task["id"])
        assert restored["state"] == "interrupted", restored
        assert restored["session_id"] == task["session_id"]
        assert len(self.invocations()) == before, "Host restart replayed a provider turn."
        self.command(action="send", id=task["id"], prompt="FIXTURE_CASE=success")
        resumed = self.settled(task["id"])
        assert resumed["state"] == "completed", resumed
        assert resumed["session_id"] == task["session_id"]
        assert self.invocations()[-1]["resume"] == task["session_id"]

    def usage(self) -> None:
        tasks = self.api("GET", "/api/coordination")["tasks"]
        completed = {task["id"] for task in tasks if task["state"] == "completed"}
        assert completed, "No completed tasks were available to verify usage."
        reported = completed | {task["id"] for task in tasks if task["messages"][0]["text"] in (
            "FIXTURE_CASE=failed_exit", "FIXTURE_CASE=stalled_exit")}
        entries = [json.loads(line) for line in (self.workbench / "usage.jsonl").read_text().splitlines()]
        turns = [entry["value"] for entry in entries if entry["kind"] == "turn"]
        for task_id in reported:
            records = [turn for turn in turns if turn["thread_id"] == task_id]
            assert len(records) == 1, (task_id, records)
            record = records[0]
            assert (record["input"], record["cache_read"], record["output"]) == (13, 7, 3), record
        assert all(turn["model"] == "codex/fixture-model" for turn in turns), turns
        usage = self.api("GET", "/api/agent-extensions/usage")
        assert not usage["errors"], usage["errors"]
        assert set(usage["by_model"]) == {"codex/fixture-model"}, usage
        total = usage["by_model"]["codex/fixture-model"]
        assert total["turns"] == total["unpriced_turns"] == len(turns), total
        assert total["estimated_usd"] is None, total
        for field in ("input", "output", "cache_read", "cache_write"):
            assert total[field] == sum(turn[field] for turn in turns), (field, total)

    def close(self) -> None:
        try:
            self.stop()
        finally:
            for call in self.invocations():
                if self.owned_worker_alive(call["pid"]):
                    os.kill(call["pid"], signal.SIGKILL)
            self.server_log.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    # Match the existing Workbench/capture request observation deadline.
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument("--label", default="workbench-functional-acceptance")
    args = parser.parse_args()
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("--timeout must be positive and finite")
    if os.name != "posix":
        parser.error("The owned process fixture requires macOS or Linux (POSIX signals and FIFOs).")
    output = args.output or ROOT / "target/workbench-evaluation" / datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    campaign = Campaign(args.server.resolve(), output, args.timeout)
    sources_before = {str(path.relative_to(ROOT)): digest(path) for path in SOURCE_INPUTS}
    results = []
    planned = ("completed", "failed_exit", "truncated", "stalled_exit", "delegation", "cancellation", "recovery", "usage")
    fatal_error = cleanup_error = None
    try:
        campaign.start()
        for name in planned:
            try:
                getattr(campaign, name)()
                results.append({"scenario": name, "passed": True})
            except Exception:
                results.append({"scenario": name, "passed": False, "error": traceback.format_exc()})
            print(f"{name}: {'PASS' if results[-1]['passed'] else 'FAIL'}", flush=True)
    except Exception:
        fatal_error = traceback.format_exc()
    finally:
        try:
            campaign.close()
        except Exception:
            cleanup_error = traceback.format_exc()
        report = {"label": args.label, "scope": "Real server/API/process/store path with an owned synthetic Codex; no model quality or live-provider claim.",
                  "server_sha256": digest(campaign.server), "planned_scenarios": planned,
                  "results": results, "fatal_error": fatal_error, "cleanup_error": cleanup_error,
                  "source_sha256": sources_before,
                  "scoped_sources_unchanged": sources_before == {str(path.relative_to(ROOT)): digest(path) for path in SOURCE_INPUTS},
                  "binary_source_binding": "The executable is copied and hashed. Source hashes describe the checkout observed during this run; retain the preceding build command/log to bind it to that executable."}
        (campaign.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        print(f"Evidence: {campaign.output / 'report.json'}", flush=True)
    return 0 if len(results) == len(planned) and all(result["passed"] for result in results) and not fatal_error and not cleanup_error and report["scoped_sources_unchanged"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
