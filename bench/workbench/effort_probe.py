"""Observe Workbench effort in actual native Codex rollouts, without coding."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import traceback
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from bench.live.run import provider_wrapper
from bench.repository.run import audit_runtime
from bench.workbench.run import Campaign, digest


def contexts(home: Path) -> list[dict]:
    return [
        {"rollout": str(file), "effort": row["payload"].get("effort")}
        for file in sorted((home / "codex/sessions").rglob("*.jsonl"))
        for line in file.read_text().splitlines()
        if (row := json.loads(line)).get("type") == "turn_context"
    ]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    plan = json.loads((args.fixture / "campaign.json").read_text())
    timeout = plan["timeout_seconds_per_provider_and_grading_step"]
    native = shutil.which("codex")
    auth = Path(os.environ.get("CODEX_HOME", Path.home() / ".codex")) / "auth.json"
    if native is None or not auth.is_file():
        parser.error("Installed Codex and its existing sign-in are required.")
    report = {
        "scope": "Configuration propagation only; no coding-quality or competitor conclusion.",
        "plan": "Old and current server each receive high against a low native CLI default. Current task then changes to low, restarts without running, resumes at low, clears the override and resumes at native default low.",
        "model": "codex/" + plan["model"],
        "timeout_seconds": timeout,
        "budget_basis": plan["budget_basis"],
        "codex_version": subprocess.check_output([native, "--version"], text=True).strip(),
        "codex_sha256": digest(Path(native).resolve()),
        "provider_config": plan["provider_config"],
        "results": [],
    }
    result_path = args.output / "report.json"

    def save() -> None:
        result_path.write_text(json.dumps(report, indent=2) + "\n")

    save()
    for label, server in (("before", args.before), ("after", args.after)):
        campaign = Campaign(server.resolve(), args.output / label, timeout)
        link = campaign.home / "codex/auth.json"
        result = {"label": label, "server_sha256": digest(campaign.server), "steps": []}
        report["results"].append(result)
        try:
            link.parent.mkdir()
            link.symlink_to(auth.resolve())
            campaign.env["BRUHS_MAIL_DB"] = str(campaign.home / "mail.sqlite3")
            campaign.env["BRUHS_REACTIVE_DB"] = str(campaign.home / "reactive.sqlite3")
            campaign.env["BRUHS_DELIVERY_DIR"] = str(campaign.home / "delivery")
            provider_wrapper(Path(native).resolve(), campaign.home / "bin/codex", campaign.calls,
                             "low", plan["provider_config"], no_daemon=True)
            campaign.start()
            prompt = "Reply OK. Do not use tools, read files, modify anything, delegate, publish or contact anyone."
            snapshot = campaign.command(action="create", parent_id=None, task={
                "title": "Native effort probe", "project_path": str(campaign.project),
                "model": report["model"], "effort": "high", "prompt": prompt,
                "permission_mode": "acceptEdits", "isolation": {"kind": "shared"},
            })
            task = campaign.settled(snapshot["tasks"][0]["id"])
            result["steps"].append({"kind": "first_turn", "state": task["state"],
                                     "saved_effort": task.get("effort"),
                                     "contexts": contexts(campaign.home)})
            assert task["state"] == "completed", task.get("error")
            expected = "low" if label == "before" else "high"
            assert [row["effort"] for row in contexts(campaign.home)] == [expected]
            assert task.get("effort") == (None if label == "before" else "high")
            save()
            if label == "after":
                task_id = task["id"]
                session = task["session_id"]
                campaign.command(action="set_effort", id=task_id, effort="low")
                assert len(campaign.invocations()) == 1
                campaign.stop()
                campaign.start()
                assert campaign.task(task_id)["effort"] == "low"
                assert len(campaign.invocations()) == 1
                for effort, followup in (("low", "Reply OK for the second turn. Use no tools."),
                                         (None, "Reply OK for the third turn. Use no tools.")):
                    campaign.command(action="set_effort", id=task_id, effort=effort)
                    campaign.command(action="send", id=task_id, prompt=followup)
                    task = campaign.settled(task_id)
                    assert task["state"] == "completed", task.get("error")
                    assert task["session_id"] == session
                    observed = contexts(campaign.home)
                    assert observed[-1]["effort"] == "low", observed
                    result["steps"].append({"kind": "resumed_turn", "saved_effort": task.get("effort"),
                                             "state": task["state"], "contexts": observed})
                    save()
                assert len(campaign.invocations()) == 3
            result["runtime_audit"] = audit_runtime(campaign.home / "codex", campaign.project)
            assert result["runtime_audit"]["passed"]
            assert not subprocess.check_output(["/usr/bin/git", "status", "--porcelain"],
                                               cwd=campaign.project, env=campaign.env)
            result["passed"] = True
        except (AssertionError, OSError, RuntimeError, ValueError, KeyError, StopIteration, subprocess.SubprocessError):
            result["passed"] = False
            result["error"] = traceback.format_exc()
        finally:
            try:
                campaign.stop()
            except (OSError, RuntimeError, subprocess.SubprocessError) as error:
                result["passed"] = False
                result["cleanup_error"] = str(error)
            campaign.server_log.close()
            if link.is_symlink():
                link.unlink()
            result["credential_link_removed"] = not os.path.lexists(link)
            save()
        print(json.dumps({"label": label, "passed": result["passed"]}), flush=True)
        if not result["passed"]:
            break
    report["passed"] = len(report["results"]) == 2 and all(row["passed"] for row in report["results"])
    save()
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
