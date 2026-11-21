"""Check real GitHub read/proposal routing without allowing forge mutations."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import traceback


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from bench.workbench.run import Campaign, digest  # noqa: E402


def readonly_gh(binary: Path, wrapper: Path, calls: Path, auth_home: Path) -> None:
    wrapper.write_text(f'''#!{sys.executable}
import json, os, sys
args = sys.argv[1:]
allowed = len(args) > 1 and ((args[0] == "pr" and args[1] in ("view", "list", "diff")) or
          (args[0] == "api" and args[1].startswith("repos/{{owner}}/{{repo}}/pulls/")))
for arg in args:
    if arg.startswith(("-X", "-f", "-F")) or arg.split("=", 1)[0] in ("--method", "--raw-field", "--field", "--input"):
        allowed = False
with open({str(calls)!r}, "a", encoding="utf-8") as log:
    log.write(json.dumps({{"args": args, "repository": os.environ.get("GH_REPO"),
                          "host": os.environ.get("GH_HOST"), "read_only_allowed": allowed}}) + "\\n")
if not allowed:
    raise SystemExit("Read-only delivery check refused an unexpected GitHub command.")
# gh's keyring lookup needs its original home; application stores stay private.
os.environ["HOME"] = {str(auth_home)!r}
os.execv({str(binary)!r}, [{str(binary)!r}, *args])
''', encoding="utf-8")
    wrapper.chmod(0o700)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--origin", required=True, help="GitHub OWNER/REPO whose PR is expected")
    parser.add_argument("--other", required=True, help="A different authenticated gh default repository")
    parser.add_argument("--number", type=int, required=True, help="Existing PR number in both repositories")
    args = parser.parse_args()
    if args.number <= 0 or args.origin == args.other:
        parser.error("Choose a positive PR number and two different repositories.")
    for repo in (args.origin, args.other):
        if len(repo.split("/")) != 2 or not all(part and all(c.isascii() and (c.isalnum() or c in "-_.") for c in part)
                                               for part in repo.split("/")):
            parser.error("Repositories must use GitHub OWNER/REPO form.")
    gh = shutil.which("gh")
    if gh is None:
        parser.error("Install and authenticate gh before this read-only check.")
    campaign = Campaign(args.server.resolve(), args.output.resolve(), 60)
    sources = [Path(__file__), ROOT / "bench/workbench/run.py", ROOT / "bruhs-core/src/delivery.rs",
               *(ROOT / "bruhs-core/src/delivery").rglob("*.rs")]
    hashes = {str(path.relative_to(ROOT)): digest(path) for path in sources}
    for path in sources:
        target = campaign.output / "source" / path.relative_to(ROOT)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, target)
    calls = campaign.output / "gh-invocations.jsonl"
    wrapper = campaign.home / "bin/gh"
    readonly_gh(Path(gh), wrapper, calls, Path.home())
    campaign.env["PATH"] += ":/opt/homebrew/bin"
    campaign.env["GH_CONFIG_DIR"] = os.environ.get("GH_CONFIG_DIR", str(Path.home() / ".config/gh"))
    campaign.env["GH_REPO"] = args.other
    campaign.env["GH_HOST"] = "github.invalid"
    campaign.env["BRUHS_DELIVERY_DIR"] = str(campaign.home / "delivery")
    report = {"scope": "Real server, Git, gh, authenticated GitHub reads, local proposals and restart; forge writes are blocked by the owned CLI guard.",
              "started_at": datetime.now(timezone.utc).isoformat(), "server_sha256": digest(campaign.server),
              "gh_version": subprocess.run([gh, "--version"], capture_output=True, text=True, check=True, timeout=60).stdout.splitlines()[0],
              "gh_sha256": digest(Path(gh).resolve()), "origin_repository": args.origin,
              "other_repository": args.other, "pull_number": args.number,
              "source_sha256": hashes, "planned_cases": ["inherited_selectors", "different_saved_default", "rest_placeholder_scope",
                  "preview_matches_read", "prepared_receipt_restart", "read_only_guard"],
              "results": [], "fatal_error": None, "cleanup_error": None,
              "limits": ["No PR/review/comment/push/merge was submitted; full live delivery remains open.",
                         "One authenticated github.com installation; enterprise hosts and writes use isolated process fixtures.",
                         "Synthetic local Git contents with real remote metadata; no coding-quality or competitor comparison."]}
    expected_url = f"https://github.com/{args.origin}/pull/{args.number}"

    def record(name: str, passed: bool, **details: object) -> None:
        report["results"].append({"case": name, "passed": passed, **details})
        print(f"{name}: {'PASS' if passed else 'FAIL'}", flush=True)

    def git(*arguments: str) -> None:
        subprocess.run(["/usr/bin/git", "-C", str(campaign.project), *arguments],
                       env=campaign.env, capture_output=True, check=True, timeout=60)

    def read(query: str) -> dict:
        return campaign.api("POST", "/api/delivery/read", {"path": str(campaign.project),
                            "query": {"type": query, "number": args.number}})

    try:
        git("remote", "add", "origin", f"https://github.com/{args.origin}.git")
        campaign.start()
        first = read("pull")
        (campaign.output / "origin-pull.json").write_text(json.dumps(first, indent=2) + "\n")
        record("inherited_selectors", first["pull"]["url"] == expected_url, observed=first["pull"]["url"], expected=expected_url)
        git("remote", "add", "upstream", f"https://github.com/{args.other}.git")
        git("config", "remote.upstream.gh-resolved", "base")
        second = read("pull")
        (campaign.output / "default-pull.json").write_text(json.dumps(second, indent=2) + "\n")
        record("different_saved_default", second["pull"]["url"] == expected_url, observed=second["pull"]["url"], expected=expected_url)
        comments = read("line_comments")
        (campaign.output / "line-comments.json").write_text(json.dumps(comments, indent=2) + "\n")
        observations = [json.loads(line) for line in calls.read_text().splitlines()]
        api = [call for call in observations if call["args"][0] == "api"]
        record("rest_placeholder_scope", bool(api) and all(call["repository"] == f"github.com/{args.origin}" and call["host"] == "github.com" for call in api),
               observed_contexts=api, body_limit="An empty comment list alone cannot establish repository scope; child-process selectors are checked too.")
        preview = campaign.api("POST", "/api/delivery/prepare", {"path": str(campaign.project), "timeout_seconds": 60,
            "action": {"type": "review", "number": args.number, "head_oid": second["pull"]["head_oid"],
                       "event": "comment", "body": "Read-only routing check: local preview only, never executed.", "comments": []}})
        (campaign.output / "preview.json").write_text(json.dumps(preview, indent=2) + "\n")
        record("preview_matches_read", preview["repository"]["project"] == args.origin and second["pull"]["url"] == expected_url,
               frozen_repository=preview["repository"], observed_pull=second["pull"]["url"])
        count = len(observations)
        campaign.stop()
        campaign.start()
        restored = campaign.api("GET", "/api/delivery")
        receipt = next(item for item in restored["actions"] if item["prepared"]["id"] == preview["id"])
        after = [json.loads(line) for line in calls.read_text().splitlines()]
        record("prepared_receipt_restart", receipt["status"] == "prepared" and receipt["completed_commands"] == 0
               and receipt["prepared"] == preview and len(after) == count)
    except Exception:
        report["fatal_error"] = traceback.format_exc()
    finally:
        try:
            campaign.close()
        except Exception:
            report["cleanup_error"] = traceback.format_exc()
        observations = [json.loads(line) for line in calls.read_text().splitlines()] if calls.exists() else []
        record("read_only_guard", bool(observations) and all(call["read_only_allowed"] for call in observations))
        finished = {row["case"] for row in report["results"]}
        report["results"].extend({"case": name, "passed": False, "not_run": "Campaign ended before this case."}
                                 for name in report["planned_cases"] if name not in finished)
        report["sources_unchanged"] = hashes == {str(path.relative_to(ROOT)): digest(path) for path in sources}
        (campaign.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        print(f"Evidence: {campaign.output / 'report.json'}", flush=True)
    return 0 if (all(row["passed"] for row in report["results"]) and not report["fatal_error"]
                 and not report["cleanup_error"] and report["sources_unchanged"]) else 1


if __name__ == "__main__":
    sys.exit(main())
