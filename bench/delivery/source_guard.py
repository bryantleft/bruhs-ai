"""Exercise delivery review fingerprints through isolated HTTP and browser routes."""
from __future__ import annotations

import argparse
import json
import math
import shlex
import shutil
import subprocess
import sys
import traceback
from pathlib import Path

from bench.workbench.run import ROOT, Campaign, digest
from bench.workbench.verification_review import save


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument("--legacy-negative-control", action="store_true")
    parser.add_argument("--skip-browser", action="store_true")
    args = parser.parse_args()
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("--timeout must be positive and finite")
    if args.legacy_negative_control and not args.skip_browser:
        parser.error("The old-server negative control is HTTP-only; use --skip-browser.")
    campaign = Campaign(args.server.resolve(), args.output, args.timeout)
    for name in ("BRUHS_MAIL_DB", "BRUHS_REACTIVE_DB", "BRUHS_DELIVERY_DIR"):
        campaign.env[name] = str(campaign.home / name.lower())
    git = shutil.which("git")
    assert git is not None
    subprocess.run([git, "-C", str(campaign.project), "config", "remote.origin.url",
                    "https://github.com/owned-fixture/no-network.git"], env=campaign.env,
                   capture_output=True, check=True)
    git_log = campaign.output / "git-calls.jsonl"
    forge_log = campaign.output / "forge-calls.jsonl"
    campaign.env.update(BRUHS_GUARD_REAL_GIT=git, BRUHS_GUARD_GIT_LOG=str(git_log))
    proxy = campaign.output / "fixture_git.py"
    shutil.copy2(Path(__file__).with_name("fixture_git.py"), proxy)
    binary_dir = campaign.home / "bin"
    (binary_dir / "git").write_text(
        f'#!/bin/sh\nexec {shlex.quote(sys.executable)} {shlex.quote(str(proxy))} "$@"\n')
    (binary_dir / "git").chmod(0o700)
    # Never delegate to installed forge clients, even if a reviewed command gets
    # past the Git proxy due to a product regression.
    for program in ("gh", "glab"):
        stub = binary_dir / program
        stub.write_text(f'#!/bin/sh\nprintf "%s\\n" blocked >> {shlex.quote(str(forge_log))}\nexit 98\n')
        stub.chmod(0o700)
    sources = [Path(__file__), Path(__file__).with_name("fixture_git.py"),
               ROOT / "bruhs-web/tests/delivery_source_guard_browser.mjs",
               ROOT / "bruhs-core/src/delivery.rs", ROOT / "bruhs-core/src/delivery/plan.rs",
               ROOT / "bruhs-core/src/delivery/fingerprint_tests.rs",
               ROOT / "bruhs-server/src/routes/delivery_ui.html"]
    before = {str(path.relative_to(ROOT)): digest(path) for path in sources}
    for path in sources:
        copy = campaign.output / "source" / path.relative_to(ROOT)
        copy.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, copy)
    steps: list[dict[str, object]] = []
    report = {
        "scope": "Owned private repository, real local delivery HTTP and optional browser UI. All Git writes and forge commands are blocked before execution; no actual commit, push or pull request occurs.",
        "profile": "legacy-negative-control" if args.legacy_negative_control else "candidate",
        "server_sha256": digest(campaign.server), "timeout_seconds": args.timeout,
        "source_sha256_before": before, "steps": steps, "fatal_error": None,
        "cleanup_error": None, "passed": False,
    }
    path = campaign.output / "report.json"

    def checkpoint(label: str, evidence: object) -> None:
        steps.append({"label": label, "passed": True, "evidence": evidence})
        save(path, report)

    def git_state() -> dict:
        return {"head": subprocess.check_output([git, "-C", str(campaign.project), "rev-parse", "HEAD"],
                                                 env=campaign.env, text=True).strip(),
                "index_sha256": digest(campaign.project / ".git/index")}

    save(path, report)
    finished = False
    try:
        (campaign.project / "a").write_bytes(b"b")
        (campaign.project / "b").write_bytes(b"c")
        initial = git_state()
        campaign.start()
        prepared = campaign.api("POST", "/api/delivery/prepare", {
            "forge": "git_hub", "path": str(campaign.project), "timeout_seconds": 60,
            "action": {"type": "commit_push_pull", "files": ["a", "b"],
                       "message": "Owned source guard fixture", "title": "Owned source guard fixture",
                       "body": "No external delivery is permitted by the fixture.",
                       "base": "release", "head": "main", "draft": True},
        })
        save(campaign.output / "prepared.json", prepared)
        assert git_state() == initial
        checkpoint("Preparation persists a reviewed proposal without changing HEAD or index", {
            "id": prepared["id"], "fingerprint": prepared["worktree_fingerprint"], **initial})
        # Identical concatenated path/payload bytes under the old unframed hash.
        (campaign.project / "a").write_bytes(b"")
        (campaign.project / "b").write_bytes(b"bc")
        campaign.api("POST", "/api/delivery/execute", {"id": prepared["id"]})

        def settled() -> dict | None:
            action = next(item for item in campaign.api("GET", "/api/delivery")["actions"]
                          if item["prepared"]["id"] == prepared["id"])
            return action if action["status"] not in {"prepared", "running"} else None

        receipt = campaign.wait(settled, "source guard receipt settlement")
        save(campaign.output / "receipt.json", receipt)
        calls = [json.loads(line) for line in git_log.read_text().splitlines()]
        blocked = [call for call in calls if not call["allowed"]]
        rejected = "changed after review" in (receipt["error"] or "")
        assert receipt["status"] == "failed" and receipt["completed_commands"] == 0, receipt
        if args.legacy_negative_control:
            assert not prepared["worktree_fingerprint"].startswith("v2:")
            assert not rejected and len(blocked) == 1 and blocked[0]["command"] == ["add", "--", "a", "b"], blocked
        else:
            assert prepared["worktree_fingerprint"].startswith("v2:")
            assert rejected and not blocked, {"receipt": receipt, "blocked": blocked}
        assert not forge_log.exists(), "A forge command was attempted"
        assert git_state() == initial, "The owned repository changed during the failed action"
        assert (campaign.project / "a").read_bytes() == b"" and (campaign.project / "b").read_bytes() == b"bc"
        checkpoint("Changed payload is rejected before any Git write" if rejected else
                   "Old fingerprint bypass reaches a blocked Git add in the owned negative control", {
                       "guard_rejected_change": rejected, "blocked_git_calls": blocked,
                       "forge_commands_attempted": 0, "receipt": receipt,
                       "head_and_index_unchanged": True, "changed_payload_preserved": True})
        if not args.skip_browser:
            save(campaign.output / "browser-input.json", {
                "base": f"http://127.0.0.1:{campaign.port}", "project": str(campaign.project),
                "id": prepared["id"], "error": receipt["error"], "timeout_ms": args.timeout * 1000})
            with (campaign.output / "browser.log").open("w") as log:
                result = subprocess.run(["node", str(ROOT / "bruhs-web/tests/delivery_source_guard_browser.mjs"),
                                         str(campaign.output / "browser-input.json"), str(campaign.output)],
                                        stdout=log, stderr=subprocess.STDOUT, check=False)
            assert result.returncode == 0, "Browser acceptance failed; inspect browser.log"
            browser = json.loads((campaign.output / "browser-report.json").read_text())
            assert browser["passed"], browser
            checkpoint("Desktop and mobile receipts explain that a fresh review is required", browser)
        finished = True
    except (AssertionError, OSError, RuntimeError, ValueError, KeyError, subprocess.SubprocessError):
        finished = False
        report["fatal_error"] = traceback.format_exc()
    finally:
        try:
            campaign.stop()
        except (AssertionError, OSError, RuntimeError, ValueError, KeyError, subprocess.SubprocessError):
            report["cleanup_error"] = traceback.format_exc()
        campaign.server_log.close()
        after = {str(path.relative_to(ROOT)): digest(path) for path in sources}
        report["source_sha256_after"] = after
        report["scoped_sources_unchanged"] = before == after
        report["server_unchanged"] = digest(campaign.server) == report["server_sha256"]
        report["passed"] = (finished and report["cleanup_error"] is None
                            and report["scoped_sources_unchanged"] and report["server_unchanged"])
        save(path, report)
    print(json.dumps({"passed": report["passed"], "profile": report["profile"], "report": str(path)}))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
