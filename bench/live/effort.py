"""Verify an API-selected effort overrides the same low Codex default before/after a fix."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import traceback


sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from bench.live import run as pilot  # noqa: E402


def probe(server: Path, output: Path, requested: str, expected: str, model: str) -> dict:
    auth = Path(os.environ.get("CODEX_HOME", Path.home() / ".codex")) / "auth.json"
    binary = shutil.which("codex")
    if binary is None or not auth.is_file():
        raise RuntimeError("Codex CLI and an authenticated auth.json are required.")
    campaign = pilot.Campaign(server.resolve(), output.resolve(), 60)
    try:
        with tempfile.TemporaryDirectory(prefix="bruhs-effort-auth-") as temporary:
            home = Path(temporary)
            (home / "auth.json").symlink_to(auth.resolve())
            sessions = campaign.output / "sessions"
            sessions.mkdir()
            (home / "sessions").symlink_to(sessions)
            campaign.env["CODEX_HOME"] = str(home)
            campaign.env["PATH"] += ":/opt/homebrew/bin"
            campaign.env.pop("BRUHS_FIXTURE_LOG", None)
            pilot.provider_wrapper(Path(binary).resolve(), campaign.home / "bin/codex",
                                   campaign.output / "provider-calls.jsonl", "low")
            campaign.start()
            body = {"prompt": "Reply exactly EFFORT_PROBE_OK. Do not use any tools.",
                    "project_path": str(campaign.project), "thread_id": "effort-probe",
                    "model": f"codex/{model}", "permission_mode": "acceptEdits",
                    "thread_access": "off", "fresh_session": True, "effort": requested}
            response = subprocess.run([
                "/usr/bin/curl", "--silent", "--show-error", "--fail-with-body", "--max-time", "60",
                "-H", "Content-Type: application/json", "--data-binary", json.dumps(body),
                f"http://127.0.0.1:{campaign.port}/api/agent",
            ], cwd=campaign.home, env=campaign.env, capture_output=True, timeout=campaign.timeout)
            log = campaign.output / "turn.log"
            log.write_bytes(response.stdout)
            (campaign.output / "turn.err").write_bytes(response.stderr)
            rows = pilot.read_jsonl(str(log))
            events = [row for row in rows if row.get("envelope") == "query"]
            results = [row for row in events if row.get("type") == "result"]
            contexts = []
            for path in sessions.rglob("*.jsonl"):
                for line in path.read_text(encoding="utf-8").splitlines():
                    row = json.loads(line)
                    if row.get("type") == "turn_context":
                        value = row["payload"]
                        contexts.append({key: value.get(key) for key in
                                         ("model", "effort", "approval_policy", "sandbox_policy")})
            passed = (response.returncode == 0 and len(results) == 1
                      and results[0].get("is_error") is False
                      and results[0].get("result", "").strip() == "EFFORT_PROBE_OK"
                      and bool(contexts) and all(value["model"] == model and value["effort"] == expected
                                               for value in contexts))
            return {"passed": passed, "requested_effort": requested, "cli_default": "low",
                    "expected_effort": expected, "observed_contexts": contexts,
                    "http_client_exit": response.returncode, "completion_count": len(results),
                    "server_sha256": pilot.digest(campaign.server)}
    finally:
        campaign.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline-server", type=Path, required=True)
    parser.add_argument("--server", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--model", required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    report = {"scope": "Live API/SDK/CLI effort propagation; no reasoning-quality comparison.",
              "model": args.model, "results": [], "fatal_error": None,
              "source_sha256": pilot.digest(Path(__file__))}
    try:
        for label, server, expected in (("before", args.baseline_server, "low"), ("after", args.server, "max")):
            result = probe(server, args.output / label, "max", expected, args.model)
            report["results"].append({"version": label, **result})
            print(f"{label}: {'PASS' if result['passed'] else 'FAIL'}", flush=True)
    except Exception:
        report["fatal_error"] = traceback.format_exc()
    finally:
        (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return 0 if not report["fatal_error"] and len(report["results"]) == 2 and all(row["passed"] for row in report["results"]) else 1


if __name__ == "__main__":
    sys.exit(main())
