"""Isolated real HTTP/process/UI QA for owner-set budgets, using synthetic Codex only.

Example: python3 -B -m bench.workbench.execution_budget --server /frozen/bruhs-server
All output stays in target/completion-budget-evaluation. Existing runs are never overwritten.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import http.client
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import traceback
import uuid

from bench.workbench.run import ACTIVE, Campaign, ROOT, digest

OUTPUT_ROOT = ROOT / "target/completion-budget-evaluation"
PLAYWRIGHT = "/Users/bryantle/.npm/_npx/9833c18b2d85bc59/node_modules/playwright"
SOURCES = [Path(__file__), Path(__file__).with_name("fixture_budget_codex.py"),
           ROOT / "bruhs-web/tests/execution_budget_browser.mjs", ROOT / "bruhs-core/src/agent.rs",
           ROOT / "bruhs-core/src/agent/budget.rs", ROOT / "bruhs-core/src/coordination.rs",
           ROOT / "bruhs-models/src/coordination.rs", ROOT / "bruhs-server/src/routes/agent.rs",
           ROOT / "bruhs-server/src/routes/coordination/remote.html", ROOT / "codex-sdk-rs/src/query.rs",
           ROOT / "bruhs-web/fonts/MonaspaceNeonVariable.woff2", ROOT / "Cargo.lock"]


def atomic_write(path: Path, content: bytes | str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}-", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as handle:
            handle.write(content.encode() if isinstance(content, str) else content)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


def save_json(path: Path, value: object) -> None:
    atomic_write(path, json.dumps(value, indent=2, sort_keys=True) + "\n")


GUARD = r"""import fcntl,json,os,subprocess,sys,tempfile
from pathlib import Path
args=sys.argv[1:]; command=args[:]; cwd=Path.cwd().resolve()
while command and command[0] in ['-C','-c'] and len(command)>2:
    if command[0]=='-C': cwd=(cwd/command[1]).resolve()
    command=command[2:]
private=Path(os.environ['BRUHS_BUDGET_PRIVATE']).resolve()
verb=command[0] if command else ''
inside=cwd.is_relative_to(private)
forbidden=verb in {'push','fetch','pull','send-pack','receive-pack','ls-remote','credential','submodule'}
allowed=inside and not forbidden
row={'cwd':str(cwd),'args':args,'allowed':allowed,'synthetic_guard':True}
path=Path(os.environ['BRUHS_BUDGET_GIT_LOG'])
with path.with_suffix('.lock').open('a') as lock:
    fcntl.flock(lock,fcntl.LOCK_EX)
    previous=path.read_bytes() if path.exists() else b''
    fd,tmp=tempfile.mkstemp(prefix='.'+path.name,dir=path.parent)
    with os.fdopen(fd,'wb') as handle:
        handle.write(previous+(json.dumps(row)+'\n').encode()); handle.flush(); os.fsync(handle.fileno())
    os.replace(tmp,path)
if not allowed:
    print('Owned budget QA blocked external or nonprivate Git operation.',file=sys.stderr); sys.exit(97)
sys.exit(subprocess.run([os.environ['BRUHS_BUDGET_REAL_GIT'],*args],check=False).returncode)
"""


class BudgetCampaign(Campaign):
    def __init__(self, server: Path, output: Path, timeout: float, negative: bool, provenance: Path | None) -> None:
        # Existing Campaign supplies the private HOME/repository, HTTP helpers, and owned server.
        super().__init__(server, output, timeout)
        atomic_write(self.worker, Path(__file__).with_name("fixture_budget_codex.py").read_bytes())
        binary_dir = self.home / "bin"
        atomic_write(binary_dir / "codex", f'#!/bin/sh\nexec {shlex.quote(sys.executable)} {shlex.quote(str(self.worker))} "$@"\n')
        (binary_dir / "codex").chmod(0o700)
        guard = self.output / "fixture_git_guard.py"
        atomic_write(guard, GUARD)
        atomic_write(binary_dir / "git", f'#!/bin/sh\nexec {shlex.quote(sys.executable)} {shlex.quote(str(guard))} "$@"\n')
        (binary_dir / "git").chmod(0o700)
        for forge in ("gh", "glab", "capy", "claude", "cursor", "opencode", "antigravity", "grok"):
            atomic_write(binary_dir / forge, '#!/bin/sh\nprintf "%s\\n" "Owned budget QA blocks this executable." >&2\nexit 97\n')
            (binary_dir / forge).chmod(0o700)
        self.env.update({"BRUHS_BUDGET_PRIVATE": str(self.home), "BRUHS_BUDGET_REAL_GIT": shutil.which("git") or "/usr/bin/git",
                         "BRUHS_BUDGET_GIT_LOG": str(self.output / "git-guard.jsonl")})
        # Serve the shared UI's actual local font without depending on a mutable SPA build.
        web_dist = self.home / "minimal-web-dist"
        atomic_write(web_dist / "fonts/MonaspaceNeonVariable.woff2", (ROOT / "bruhs-web/fonts/MonaspaceNeonVariable.woff2").read_bytes())
        self.env["BRUHS_WEB_DIST"] = str(web_dist)
        self.report = {"passed": False, "profile": "http-only-negative-control" if negative else "final-http-process-and-browser",
                       "scope": "Real local HTTP, owned processes, private files, and shared Tasks browser UI. All Codex protocol/text/usage is synthetic. No model quality, paid usage, or sandbox-enforcement claim.",
                       "checks": [], "cases": [], "limitations": ["Codex cannot produce a real approval request; pending-approval and background registry settlement require the core tests.", "HTTP/browser observation uses fixture budgets, not performance benchmarks."]}
        self.negative = negative
        self.saved_tasks: list[dict] = []
        manifest = {"frozen_binary_origin": str(server.resolve()), "binary_sha256": digest(self.server),
                    "snapshot_at_utc": datetime.now(timezone.utc).isoformat(), "source_snapshot": {},
                    "provenance_note": "Source snapshot and binary identity are recorded independently; a provided parent manifest establishes build provenance."}
        for source in SOURCES:
            if source.is_file():
                destination = self.output / "source" / source.relative_to(ROOT)
                atomic_write(destination, source.read_bytes())
                manifest["source_snapshot"][str(source.relative_to(ROOT))] = digest(destination)
        if provenance:
            atomic_write(self.output / "provided-provenance", provenance.read_bytes())
            manifest["provided_provenance_sha256"] = digest(provenance)
        save_json(self.output / "manifest.json", manifest)

    def check(self, name: str, actual: object, expected: object = True) -> None:
        passed = actual == expected
        self.report["checks"].append({"name": name, "passed": passed, "actual": actual, "expected": expected})
        if not passed:
            raise AssertionError(f"{name}: {actual!r} != {expected!r}")

    def case(self, name: str, run) -> None:
        row = {"name": name, "passed": False}
        try:
            run()
            row["passed"] = True
        except Exception as error:
            row.update(error=str(error), traceback=traceback.format_exc())
        self.report["cases"].append(row)
        save_json(self.output / "report.json", self.report)
        print(json.dumps({"case": name, "passed": row["passed"], "error": row.get("error")}), flush=True)

    def providers(self) -> list[dict]:
        return [row for row in self.invocations() if row.get("role") == "provider"]

    def request(self, method: str, path: str, body: dict | None = None) -> tuple[int, object]:
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=self.timeout)
        try:
            connection.request(method, path, json.dumps(body) if body is not None else None, {"Content-Type": "application/json"})
            response = connection.getresponse()
            raw = response.read().decode()
            return response.status, json.loads(raw) if raw else None
        finally:
            connection.close()

    def agent_body(self, case: str, seconds: object = None, thread: str | None = None) -> dict:
        body = {"thread_id": thread or str(uuid.uuid4()), "project_path": str(self.project),
                "model": "codex/fixture-model", "permission_mode": "acceptEdits", "effort": "high",
                "prompt": f"SYNTHETIC owned QA only. BUDGET_FIXTURE_CASE={case}"}
        if seconds is not None:
            body["execution_timeout_seconds"] = seconds
        return body

    def sse(self, label: str, body: dict, observation: float | None = None, delayed_read: float = 0) -> dict:
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=observation or self.timeout)
        started = time.monotonic()
        result = {"request": body, "events": [], "read_timeout": False}
        wire: list[bytes] = []
        try:
            connection.request("POST", "/api/agent", json.dumps(body), {"Content-Type": "application/json"})
            response = connection.getresponse()
            result["status"] = response.status
            if delayed_read:
                time.sleep(delayed_read)
            while True:
                line = response.readline()
                if not line:
                    break
                wire.append(line)
                if line.startswith(b"data:"):
                    result["events"].append(json.loads(line[5:].strip()))
        except (TimeoutError, socket.timeout):
            result["read_timeout"] = True
        finally:
            connection.close()
            result["elapsed_seconds"] = time.monotonic() - started
            atomic_write(self.output / f"{label}.sse", b"".join(wire))
            save_json(self.output / f"{label}.json", result)
        return result

    def stopped(self, calls: list[dict]) -> None:
        pids = [pid for row in calls for pid in (row["pid"], row.get("child_pid")) if pid]
        self.wait(lambda: all(not self.owned_worker_alive(pid) for pid in pids), "owned provider and child cleanup")
        save_json(self.output / f"process-cleanup-{time.monotonic_ns()}.json", {"pids": pids, "all_stopped": True})

    def cleanup_workers(self) -> None:
        pids = {pid for row in self.invocations() for pid in (row["pid"], row.get("child_pid")) if pid}
        for sig in (signal.SIGTERM, signal.SIGKILL):
            for pid in pids:
                if self.owned_worker_alive(pid):
                    try:
                        os.kill(pid, sig)
                    except ProcessLookupError:
                        pass
            time.sleep(0.15)
        save_json(self.output / "cleanup.json", {"owned_pids": sorted(pids), "survivors": [pid for pid in pids if self.owned_worker_alive(pid)]})

    def invalid_http(self) -> None:
        before = len(self.providers())
        observations = []
        for value in (0, -1, 0.5, 18446744073709551615, 18446744073709551616):
            status, body = self.request("POST", "/api/agent", self.agent_body("success", value))
            observations.append({"route": "/api/agent", "value": value, "status": status, "body": body})
            save_json(self.output / "invalid-http.json", observations)
            self.check(f"Invalid HTTP budget {value} rejected before a provider", status, 400)
        for value in (0, 18446744073709551615):
            request = {"action": "create", "task": {"title": "SYNTHETIC rejected budget", "project_path": str(self.project),
                       "model": "codex/fixture-model", "prompt": "BUDGET_FIXTURE_CASE=success", "permission_mode": "acceptEdits",
                       "isolation": {"kind": "shared"}, "execution_timeout_seconds": value}}
            status, body = self.request("POST", "/api/coordination/commands", request)
            observations.append({"route": "/api/coordination/commands", "value": value, "status": status, "body": body})
            save_json(self.output / "invalid-http.json", observations)
            self.check(f"Invalid new-task budget {value} rejected before a provider", status, 400)
        self.check("Invalid budgets start no synthetic provider", len(self.providers()), before)

    def http_expiration(self) -> None:
        before = len(self.providers())
        body = self.agent_body("stall", 3)
        result = self.sse("http-expiration", body)
        self.check("Budgeted HTTP stream returns 200", result["status"], 200)
        self.check("Budget ends the HTTP stream within observation", result["read_timeout"], False)
        errors = [e for e in result["events"] if e.get("kind") == "execution_budget_exceeded"]
        self.check("Exactly one distinct terminal execution-budget error", len(errors), 1)
        self.check("Budget terminal envelope is query error", [errors[0].get("envelope"), errors[0].get("type"), errors[0].get("execution_timeout_seconds")], ["query", "error", 3])
        self.check("Expired turn never emits native result", any(e.get("type") == "result" for e in result["events"]), False)
        session = next(e["session_id"] for e in result["events"] if e.get("type") == "init")
        self.check("Partial synthetic text is retained before terminal", any("SYNTHETIC fixture partial output" in e.get("text", "") for e in result["events"]))
        calls = self.providers()[before:]
        self.check("One synthetic provider invocation per expired HTTP turn", len(calls), 1)
        self.check("Private partial file persists", (self.project / "budget-partial.txt").is_file())
        self.stopped(calls)
        time.sleep(0.4)
        self.check("Expiration starts no automatic retry", len(self.providers()), before + 1)
        resumed = self.sse("http-explicit-resume", self.agent_body("resume", 3, body["thread_id"]))
        self.check("Explicit continuation emits successful synthetic result", any(e.get("type") == "result" and not e.get("is_error") for e in resumed["events"]))
        self.check("HTTP continuation retains provider session", self.providers()[-1]["resume"], session)
        self.check("Latest explicit resume marker overrides previous stall context", self.providers()[-1]["case"], "resume")

    def http_normal(self) -> None:
        result = self.sse("http-default-unlimited", self.agent_body("success"))
        self.check("No-budget normal turn completes", any(e.get("type") == "result" and not e.get("is_error") for e in result["events"]))
        self.check("No-budget turn has no expiration", any(e.get("kind") == "execution_budget_exceeded" for e in result["events"]), False)
        result = self.sse("http-backpressure", self.agent_body("flood", 1), delayed_read=2.2)
        self.check("Backpressured HTTP stream still delivers the terminal budget event", sum(e.get("kind") == "execution_budget_exceeded" for e in result["events"]), 1)
        self.check("Backpressured HTTP turn never reports completion", any(e.get("type") == "result" for e in result["events"]), False)
        self.stopped(self.providers()[-1:])

    def create_budget_task(self, case: str, seconds: int | None, title: str | None = None) -> dict:
        title = title or f"SYNTHETIC budget {case} {uuid.uuid4()}"
        task = {"title": title, "project_path": str(self.project), "model": "codex/fixture-model", "effort": "high",
                "prompt": f"SYNTHETIC owned QA only. BUDGET_FIXTURE_CASE={case}", "permission_mode": "acceptEdits",
                "isolation": {"kind": "worktree", "base_branch": "main"}}
        if seconds is not None:
            task["execution_timeout_seconds"] = seconds
        result = self.command(action="create", parent_id=None, task=task)
        return next(t for t in result["tasks"] if t["title"] == title)

    def coordination_basic(self) -> None:
        created = self.create_budget_task("stall", 3)
        running = self.running(created["id"])
        self.command(action="auto_resume", id=created["id"], enabled=True)
        status, error = self.request("POST", "/api/coordination/commands", {"action": "set_execution_budget", "id": created["id"], "seconds": 4})
        save_json(self.output / "active-setter-rejection.json", {"status": status, "body": error})
        self.check("Active task budget edit is refused", status, 409)
        task = self.settled(created["id"])
        self.check("Budget-expired coordinated task is Interrupted", task["state"], "interrupted")
        self.check("Interrupted task retains initialized session", task["session_id"], running["session_id"])
        self.check("Interrupted task preserves partial assistant output", any("SYNTHETIC fixture partial output" in m["text"] for m in task["messages"]))
        self.check("Interrupted task records an actionable budget error", "execution budget" in task["error"] and "without a completion" in task["error"])
        self.check("Budget expiration is not quota Limited", task["state"] != "limited" and task["resume_at"] is None)
        calls = [c for c in self.providers() if c["workspace"] == task["working_path"]]
        self.stopped(calls)
        before = len(self.providers())
        for invalid in (0, 18446744073709551615):
            status, _ = self.request("POST", "/api/coordination/commands", {"action": "set_execution_budget", "id": task["id"], "seconds": invalid})
            self.check(f"Invalid settled budget {invalid} rejected", status, 400)
        for budget in (7, None, 3):
            self.command(action="set_execution_budget", id=task["id"], seconds=budget)
            edited = self.task(task["id"])
            self.check(f"Settled budget persists {budget}", edited.get("execution_timeout_seconds"), budget)
            for key in ("model", "permission_mode", "effort", "working_path", "project_path", "messages", "session_id", "state"):
                self.check(f"Saving budget {budget} preserves {key}", edited[key], task[key])
        self.check("Saving budgets starts no provider", len(self.providers()), before)
        self.command(action="send", id=task["id"], prompt="SYNTHETIC owner explicitly resumes. BUDGET_FIXTURE_CASE=resume")
        resumed = self.settled(task["id"])
        self.check("Explicit coordinated resume completes", resumed["state"], "completed")
        self.check("Explicit coordinated resume preserves session", resumed["session_id"], task["session_id"])
        self.check("Explicit continuation chooses latest prompt marker", self.providers()[-1]["case"], "resume")
        save_json(self.output / "coordination-basic.json", {"interrupted": task, "resumed": resumed})

    def tree_scope(self) -> None:
        unrelated = self.running(self.create_budget_task("hold", None)["id"])
        root = self.running(self.create_budget_task("stall", 8)["id"])
        completed_children = []
        explicit_request = {"title": "SYNTHETIC explicit child duration", "project_path": str(self.project),
                            "prompt": "SYNTHETIC child owned QA. BUDGET_FIXTURE_CASE=success", "model": "codex/fixture-model",
                            "permission_mode": "acceptEdits", "isolation": {"kind": "worktree", "base_branch": "main"},
                            "execution_timeout_seconds": 20}
        result = self.command(action="create", parent_id=root["id"], task=explicit_request)
        explicit = next(t for t in result["tasks"] if t["title"] == explicit_request["title"])
        self.check("Explicit child budget overrides the inherited default", explicit.get("execution_timeout_seconds"), 20)
        completed_children.append(self.settled(explicit["id"]))
        fanout_request = dict(explicit_request, title="SYNTHETIC fanout inherited duration")
        del fanout_request["execution_timeout_seconds"]
        result = self.command(action="fanout", parent_id=root["id"], task=fanout_request, models=["codex/fixture-model"])
        fanout = next(t for t in result["tasks"] if t["title"] == fanout_request["title"])
        self.check("Generic parented Fanout inherits an omitted budget", fanout.get("execution_timeout_seconds"), 8)
        completed_children.append(self.settled(fanout["id"]))
        descendants = []
        current = root
        for index in range(2):
            if index == 0:
                # Generic Create-child must inherit when the optional budget is omitted.
                result = self.command(action="create", parent_id=current["id"], task={
                    "title": f"SYNTHETIC budget descendant {index}", "project_path": str(self.project),
                    "prompt": "SYNTHETIC child owned QA. BUDGET_FIXTURE_CASE=hold", "model": "codex/fixture-model",
                    "effort": "high", "permission_mode": "acceptEdits", "isolation": {"kind": "worktree", "base_branch": "main"}})
            else:
                result = self.command(action="fork", id=current["id"], title=f"SYNTHETIC budget descendant {index}",
                                      prompt="SYNTHETIC child owned QA. BUDGET_FIXTURE_CASE=hold", relationship="fork",
                                      model="codex/fixture-model", effort="high", isolation={"kind": "worktree", "base_branch": "main"})
            child = next(t for t in result["tasks"] if t["parent_id"] == current["id"] and t["title"] == f"SYNTHETIC budget descendant {index}")
            self.check(f"Descendant {index} inherits run budget", child.get("execution_timeout_seconds"), 8)
            current = self.running(child["id"])
            descendants.append(current)
        stopped_root = self.settled(root["id"])
        self.check("Expired source task becomes Interrupted", stopped_root["state"], "interrupted")
        for child in descendants:
            self.check("Active descendant is cancelled by source budget", self.task(child["id"])["state"], "cancelled")
        self.check("Independent source tree keeps running", self.task(unrelated["id"])["state"], "running")
        for completed_child in completed_children:
            self.check("Settled descendants remain completed after source expiry", self.task(completed_child["id"])["state"], "completed")
            self.check("Settled descendant files survive source expiry", Path(completed_child["working_path"], "budget-completed.txt").is_file())
        related_paths = {t["working_path"] for t in [root, *descendants]}
        self.stopped([c for c in self.providers() if c["workspace"] in related_paths])
        self.check("Independent provider remains alive", any(self.owned_worker_alive(c["pid"]) for c in self.providers() if c["workspace"] == unrelated["working_path"]))
        self.command(action="cancel", id=unrelated["id"])
        self.stopped([c for c in self.providers() if c["workspace"] == unrelated["working_path"]])
        self.saved_tasks = [self.task(t["id"]) for t in [root, *descendants, *completed_children]]
        save_json(self.output / "tree-scope.json", {"expired_tree": self.saved_tasks, "unrelated": self.task(unrelated["id"])})

    def restart_retention(self) -> None:
        tasks = self.api("GET", "/api/coordination")["tasks"]
        for task in tasks:
            if task["state"] in ACTIVE:
                self.command(action="cancel", id=task["id"])
        self.stopped(self.providers())
        before = self.api("GET", "/api/coordination")["tasks"]
        files = {task["id"]: digest(Path(task["working_path"], "budget-partial.txt")) for task in before if Path(task["working_path"], "budget-partial.txt").is_file()}
        calls = len(self.providers())
        self.stop()
        self.start()
        for prior in before:
            restored = self.task(prior["id"])
            for key in ("state", "session_id", "messages", "execution_timeout_seconds", "error", "permission_mode", "model", "effort"):
                self.check(f"Restart retains {prior['id']} {key}", restored.get(key), prior.get(key))
            if prior["id"] in files:
                self.check("Restart retains the actual partial file", digest(Path(restored["working_path"], "budget-partial.txt")), files[prior["id"]])
        self.check("Host restart never replays a provider", len(self.providers()), calls)
        save_json(self.output / "restart.json", {"before": before, "after": self.api("GET", "/api/coordination")["tasks"], "calls_unchanged": True})

    def browser(self) -> None:
        fixture = {"base": f"http://127.0.0.1:{self.port}", "project": str(self.project), "call_log": str(self.calls),
                   "timeout_ms": int(self.timeout * 1000), "owner_control_request": str(self.output / "browser-owner-request.json"),
                   "owner_control_reply": str(self.output / "browser-owner-reply.json")}
        save_json(self.output / "browser-input.json", fixture)
        environment = dict(self.env, BRUHS_PLAYWRIGHT_PATH=PLAYWRIGHT,
                           PLAYWRIGHT_BROWSERS_PATH="/Users/bryantle/Library/Caches/ms-playwright")
        node = shutil.which("node")
        if not node:
            raise RuntimeError("Installed Node is required by the browser fallback.")
        # The browser requests only a restart of this exact owned Campaign server.
        # Atomic filesystem messages avoid adding any control endpoint to production.
        process = subprocess.Popen([node, str(ROOT / "bruhs-web/tests/execution_budget_browser.mjs"), str(self.output / "browser-input.json"), str(self.output)],
                                   env=environment, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        deadline = time.monotonic() + 180
        handled: set[str] = set()
        restarts: list[dict] = []
        stdout = stderr = ""
        collected = False
        try:
            while process.poll() is None:
                if time.monotonic() >= deadline:
                    raise TimeoutError("Browser campaign exceeded its 180s observation budget.")
                control = self.output / "browser-owner-request.json"
                if control.is_file():
                    request = json.loads(control.read_text())
                    nonce = request.get("nonce")
                    if isinstance(nonce, str) and nonce not in handled:
                        handled.add(nonce)
                        receipt = {"nonce": nonce, "operation": request.get("operation"), "passed": False,
                                   "profile": request.get("profile")}
                        try:
                            if request.get("operation") != "restart_owned_server":
                                raise ValueError("Only restart_owned_server is accepted by the owned QA control.")
                            before = len(self.providers())
                            receipt["prior_server_pid"] = self.process.pid if self.process else None
                            self.stop()
                            self.start()
                            receipt["restarted_server_pid"] = self.process.pid if self.process else None
                            receipt["provider_invocations_before"] = before
                            receipt["provider_invocations_after"] = len(self.providers())
                            receipt["restored_revision"] = self.api("GET", "/api/coordination")["revision"]
                            if receipt["provider_invocations_after"] != before:
                                raise AssertionError("Owned restart automatically replayed a provider.")
                            receipt["passed"] = True
                        except Exception as error:
                            receipt["error"] = str(error)
                            receipt["traceback"] = traceback.format_exc()
                        restarts.append(receipt)
                        save_json(self.output / "browser-owner-restarts.json", restarts)
                        save_json(self.output / "browser-owner-reply.json", receipt)
                time.sleep(0.05)
            stdout, stderr = process.communicate(timeout=1)
            collected = True
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
            if not collected:
                stdout, stderr = process.communicate(timeout=1)
            save_json(self.output / "browser-owner-restarts.json", restarts)
            atomic_write(self.output / "browser.stdout", stdout)
            atomic_write(self.output / "browser.stderr", stderr)
        self.check("Headless shared Tasks browser campaign passes", process.returncode, 0)
        self.check("Desktop and touch each exercise an owned server restart", len(restarts), 2)
        self.check("Browser-owned restarts never replay providers", all(receipt["passed"] for receipt in restarts))
        browser_report = json.loads((self.output / "browser-report.json").read_text())
        self.check("Desktop and touch browser profiles pass", browser_report["passed"])

    def negative_control(self) -> None:
        result = self.sse("negative-control-http-only", self.agent_body("stall", 1), observation=3)
        self.check("Pre-budget negative control ignores execution deadline", result["read_timeout"])
        self.check("Pre-budget negative control has no budget terminal", any(e.get("kind") == "execution_budget_exceeded" for e in result["events"]), False)
        self.check("Pre-budget negative control reached fake provider", len(self.providers()), 1)
        self.check("Pre-budget negative control emitted partial synthetic output", any("SYNTHETIC fixture partial output" in e.get("text", "") for e in result["events"]))
        self.cleanup_workers()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=Path, required=True, help="Parent-supplied immutable server; never target/debug/bruhs-server")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--timeout", type=float, default=20)
    parser.add_argument("--negative-control", action="store_true", help="Labeled HTTP-only negative control against a frozen pre-budget binary")
    parser.add_argument("--skip-browser", action="store_true")
    parser.add_argument("--provenance", type=Path)
    args = parser.parse_args()
    server = args.server.resolve()
    if server == (ROOT / "target/debug/bruhs-server").resolve() or not server.is_file():
        parser.error("Supply an existing immutable frozen server path, not the mutable debug binary.")
    output = (args.output or OUTPUT_ROOT / (datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ") + ("-negative-control" if args.negative_control else "-final"))).resolve()
    if not output.is_relative_to(OUTPUT_ROOT.resolve()) or output == OUTPUT_ROOT.resolve():
        parser.error("Campaign outputs must be a new child of target/completion-budget-evaluation.")
    campaign = BudgetCampaign(server, output, args.timeout, args.negative_control, args.provenance)
    try:
        campaign.start()
        if args.negative_control:
            campaign.case("http-only pre-budget negative control", campaign.negative_control)
        else:
            campaign.case("invalid HTTP budgets never invoke provider", campaign.invalid_http)
            campaign.case("HTTP expiration cleanup and explicit session resume", campaign.http_expiration)
            campaign.case("normal unlimited completion and output backpressure", campaign.http_normal)
            campaign.case("coordinated settlement budget editing and resume", campaign.coordination_basic)
            campaign.case("source task tree budget scope", campaign.tree_scope)
            if not args.skip_browser:
                campaign.case("shared Tasks desktop and mobile touch browser", campaign.browser)
            campaign.case("restart retention without replay", campaign.restart_retention)
        campaign.report["passed"] = all(case["passed"] for case in campaign.report["cases"])
    except Exception as error:
        campaign.report["fatal_error"] = {"message": str(error), "traceback": traceback.format_exc()}
    finally:
        try:
            campaign.stop()
        finally:
            campaign.cleanup_workers()
            campaign.server_log.close()
            save_json(campaign.output / "final-tasks.json", json.loads((campaign.workbench / "task-workspaces.json").read_text()) if (campaign.workbench / "task-workspaces.json").exists() else {})
            save_json(campaign.output / "report.json", campaign.report)
    print(json.dumps({"passed": campaign.report["passed"], "output": str(output)}), flush=True)
    return 0 if campaign.report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
