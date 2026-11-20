"""Check real Workbench review status with an owned provider protocol fixture."""

from __future__ import annotations

import argparse
import json
import math
import os
import shutil
import subprocess
import tempfile
import traceback
from pathlib import Path
from typing import NotRequired, TypedDict

from bench.workbench.run import ROOT, Campaign, digest


class Step(TypedDict):
    label: str
    passed: bool
    evidence: object


class Report(TypedDict):
    scope: str
    timeout_seconds: float
    budget_basis: str
    server_sha256: str
    source_sha256_before: dict[str, str]
    steps: list[Step]
    fatal_error: str | None
    cleanup_error: str | None
    browser_requested: bool
    source_sha256_after: NotRequired[dict[str, str]]
    scoped_sources_unchanged: NotRequired[bool]
    server_unchanged: NotRequired[bool]
    passed: NotRequired[bool]


def save(path: Path, value: object) -> None:
    with tempfile.NamedTemporaryFile("w", dir=path.parent, delete=False) as handle:
        json.dump(value, handle, indent=2)
        handle.write("\n")
        handle.flush()
        os.fsync(handle.fileno())
        staged = Path(handle.name)
    staged.replace(path)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    # Reuse the existing Workbench request observation budget, not a latency SLA.
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument("--skip-browser", action="store_true")
    args = parser.parse_args()
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("--timeout must be positive and finite")
    campaign = Campaign(args.server.resolve(), args.output, args.timeout)
    for name in ("BRUHS_MAIL_DB", "BRUHS_REACTIVE_DB", "BRUHS_DELIVERY_DIR"):
        campaign.env[name] = str(campaign.home / name.lower())
    sources = [
        Path(__file__), Path(__file__).with_name("fixture_codex.py"),
        ROOT / "bruhs-web/tests/verification_review_browser.mjs",
        ROOT / "bruhs-models/src/coordination/board.rs",
        ROOT / "bruhs-models/src/coordination/board/tests/reviews.rs",
        ROOT / "bruhs-server/src/routes/coordination/remote.html",
    ]
    before = {str(path.relative_to(ROOT)): digest(path) for path in sources}
    for path in sources:
        copy = campaign.output / "source" / path.relative_to(ROOT)
        copy.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, copy)
    report: Report = {
        "scope": "Real local HTTP, persistence, board classification and browser UI. The owned protocol fixture reports a synthetic failure; no real model or correctness grader runs.",
        "timeout_seconds": args.timeout,
        "budget_basis": "Existing Workbench request observation budget; no performance conclusion.",
        "server_sha256": digest(campaign.server),
        "source_sha256_before": before,
        "steps": [], "fatal_error": None, "cleanup_error": None,
        "browser_requested": not args.skip_browser,
    }
    report_path = campaign.output / "report.json"

    def checkpoint(label: str, evidence: object) -> None:
        report["steps"].append({"label": label, "passed": True, "evidence": evidence})
        save(report_path, report)

    def card(task_id: str) -> dict:
        board = campaign.api("GET", "/api/coordination/board")
        return next(card for card in board["cards"] if card["id"] == task_id)

    def start_review(parent: dict, title: str) -> dict:
        snapshot = campaign.command(
            action="fork", id=parent["id"], title=title,
            prompt="FIXTURE_CASE=review_findings", relationship="verify",
            model="codex/fixture-model", isolation={"kind": "shared"},
        )
        review = next(task for task in snapshot["tasks"] if task["title"] == title)
        return campaign.settled(review["id"])

    finished = False
    save(report_path, report)
    try:
        campaign.start()
        snapshot = campaign.command(
            action="save_spec", id=None, project_path=str(campaign.project),
            title="Verification review acceptance", content="The implementation passes its regression checks.",
            expected_revision=0,
        )
        spec = snapshot["specs"][0]
        snapshot = campaign.command(action="create", parent_id=None, task={
            "title": "Review status acceptance", "project_path": str(campaign.project),
            "prompt": "FIXTURE_CASE=success", "model": "codex/fixture-model",
            "permission_mode": "acceptEdits", "spec_id": spec["id"],
            "isolation": {"kind": "shared"},
        })
        parent = campaign.settled(snapshot["tasks"][0]["id"])
        assert parent["state"] == "completed", parent
        review = start_review(parent, "Review revision one")
        assert review["state"] == "completed", review
        assert review["permission_mode"] == "plan", review
        assert review["spec_revision"] == 1, review
        assert "FAIL:" in review["messages"][-1]["text"], review
        value = card(parent["id"])
        assert value["status"] == {"label": "Review ready", "tone": "action", "busy": False}, value
        assert value["verification"]["task_id"] == review["id"], value
        assert value["verification"]["freshness"] == "matches_task", value
        assert campaign.invocations()[-1]["sandbox"] == "read-only", campaign.invocations()
        checkpoint("Completed verifier with unresolved findings requires report review", value)

        entry = {"base": f"http://127.0.0.1:{campaign.port}", "parent": parent,
                 "review": review, "spec": spec, "timeout_ms": args.timeout * 1000}
        save(campaign.output / "browser-input.json", entry)
        if not args.skip_browser:
            with (campaign.output / "browser.log").open("w") as log:
                result = subprocess.run(
                    ["node", str(ROOT / "bruhs-web/tests/verification_review_browser.mjs"),
                     str(campaign.output / "browser-input.json"), str(campaign.output)],
                    stdout=log, stderr=subprocess.STDOUT, check=False,
                )
            assert result.returncode == 0, "Browser acceptance failed; inspect browser.log"
            browser = json.loads((campaign.output / "browser-report.json").read_text())
            assert browser["passed"], browser
            checkpoint("Browser review navigation, specification edits and fresh review", browser)
        else:
            campaign.command(action="save_spec", id=spec["id"], project_path=spec["project_path"],
                             title=spec["title"], content="A newer acceptance criterion.", expected_revision=1)

        # Pin an older report by changing to a different specification at the same revision.
        snapshot = campaign.command(action="save_spec", id=None, project_path=str(campaign.project),
                                    title="Replacement criteria", content="Another saved contract.", expected_revision=0)
        replacement = next(item for item in snapshot["specs"] if item["id"] != spec["id"])
        campaign.command(action="attach_spec", id=parent["id"], spec_id=replacement["id"])
        value = card(parent["id"])
        assert value["lane"] == "building" and value["status"]["label"] == "Review outdated", value
        assert value["verification"]["freshness"] == "specification_changed", value
        checkpoint("Attaching a different specification invalidates the old report", value)

        review = start_review(parent, "Replacement specification review")
        assert card(parent["id"])["status"]["label"] == "Review ready"
        before_calls = len(campaign.invocations())
        campaign.stop()
        campaign.start()
        assert len(campaign.invocations()) == before_calls, "Restart launched unrequested work"
        assert card(parent["id"])["verification"]["task_id"] == review["id"]
        checkpoint("Restart preserves the report without starting another run", card(parent["id"]))

        campaign.command(action="send", id=parent["id"], prompt="FIXTURE_CASE=success\nAddress the remaining defect.")
        assert campaign.settled(parent["id"])["state"] == "completed"
        value = card(parent["id"])
        assert value["status"]["label"] == "Review outdated", value
        assert value["verification"]["freshness"] == "task_changed", value
        checkpoint("New owner request invalidates the saved review", value)
        reviews = [call for call in campaign.invocations() if call["case"] == "review_findings"]
        assert all(call["sandbox"] == "read-only" for call in reviews), reviews
        assert all(call["workspace"] == parent["working_path"] for call in reviews), reviews
        checkpoint("Every verifier invocation requests the parent's workspace and read-only sandbox", reviews)
        finished = True
    except (AssertionError, LookupError, OSError, RuntimeError, StopIteration, subprocess.SubprocessError, TypeError, ValueError):
        report["fatal_error"] = traceback.format_exc()
    finally:
        try:
            campaign.stop()
        except (OSError, RuntimeError, subprocess.SubprocessError):
            report["cleanup_error"] = traceback.format_exc()
        campaign.server_log.close()
        report["source_sha256_after"] = {str(path.relative_to(ROOT)): digest(path) for path in sources}
        report["scoped_sources_unchanged"] = before == report["source_sha256_after"]
        report["server_unchanged"] = report["server_sha256"] == digest(campaign.server)
        report["passed"] = (finished and not report["fatal_error"] and not report["cleanup_error"]
                            and report["scoped_sources_unchanged"] and report["server_unchanged"])
        save(report_path, report)
    print(json.dumps({"passed": report["passed"], "report": str(report_path)}))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
