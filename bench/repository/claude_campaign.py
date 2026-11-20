"""Fresh constrained Claude Code/Bruhs Claude cohort; providers are products under test."""

from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
import traceback
from datetime import UTC, datetime
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / "bench/lib"))
from bench.lib.metrics import read_jsonl  # noqa: E402 - script entry point configures sibling helper imports
from bench.lib.run import stop_process_group  # noqa: E402 - script entry point configures sibling helper imports
from bench.live.run import attempt  # noqa: E402 - script entry point configures sibling helper imports
from bench.repository.go_fixture import prepare  # noqa: E402 - script entry point configures sibling helper imports
from bench.repository.go_fixture import provider_wrapper as go_wrapper  # noqa: E402 - sibling CLI helper path
from bench.repository.run import freeze  # noqa: E402 - script entry point configures sibling helper imports
from bench.workbench.run import Campaign, digest  # noqa: E402 - script entry point configures sibling helper imports


def save(path: Path, value: object) -> None:
    import tempfile

    fd, tmp = tempfile.mkstemp(dir=path.parent, prefix="." + path.name + "-")
    try:
        with os.fdopen(fd, "w") as stream:
            json.dump(value, stream, indent=2)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(tmp, path)
    finally:
        Path(tmp).unlink(missing_ok=True)


def require_actual_model(actual: object, expected: str) -> None:
    if actual != expected:
        raise RuntimeError(
            "Actual model missing or changed; retain remainder unstarted."
        )


def server_path(path: Path) -> Path:
    resolved = path.resolve()
    if any(
        resolved.is_relative_to(ROOT / "target" / profile)
        for profile in ("debug", "release")
    ):
        raise ValueError("Copy the server outside mutable Cargo build outputs first.")
    return resolved


def audit_binaries(
    server: Path, provider: Path, server_sha256: str, provider_sha256: str
) -> dict[str, bool]:
    return {
        "server_binary_unchanged": digest(server) == server_sha256,
        "provider_binary_unchanged": digest(provider) == provider_sha256,
    }


def wrapper(
    binary: Path,
    path: Path,
    calls: Path,
    real_home: Path,
    root: Path,
    output: Path,
    model: str,
) -> None:
    code = f"""#!{sys.executable}
import hashlib,json,os,pathlib,sys
args=sys.argv[1:];env=dict(os.environ,HOME={str(real_home)!r},USER={os.environ.get("USER", "")!r},LOGNAME={os.environ.get("LOGNAME", "")!r})
for name in ["CLAUDE_CONFIG_DIR","XDG_CONFIG_HOME","XDG_DATA_HOME"]:env.pop(name,None)
for name in ["ANTHROPIC_API_KEY","ANTHROPIC_AUTH_TOKEN","CLAUDECODE"]:env.pop(name,None)
if "-p" in args or "--print" in args:
 if any("bypassPermissions" in arg or "dangerously-skip-permissions" in arg for arg in args):raise SystemExit("This cohort refuses bypass permissions.")
 work=pathlib.Path.cwd();denied=[{str(root)!r},{str(output / "snapshot")!r},{str(real_home / ".ssh")!r},{str(real_home / ".config/gh")!r},{str(real_home / ".codex")!r},{str(real_home / ".claude")!r}]
 attempts=pathlib.Path({str(output / "home/attempts")!r})
 if attempts.exists():denied += [str(p) for p in attempts.glob("*/*") if p.is_dir() and p.resolve()!=work.resolve()]
 settings={{"disableAllHooks":True,"sandbox":{{"enabled":True,"autoAllowBashIfSandboxed":True,"filesystem":{{"allowWrite":[str(work)],"denyRead":denied}},"network":{{"allowedDomains":[]}}}},"permissions":{{"deny":["WebFetch","WebSearch"]}}}}
 common=["--safe-mode","--restricted","--tools","Read,Write,Edit,Glob,Grep,Bash","--disable-slash-commands","--no-chrome","--no-session-persistence","--effort","high","--settings",json.dumps(settings)]
 args[:0]=common
 row={{"cwd":str(work),"model_expected":{model!r},"effort":"high","common_flags":common[:-1],"settings":settings,"credential_source":"Existing host OAuth configuration/keychain; no credential contents retained"}}
 with open({str(calls)!r},"a") as log:log.write(json.dumps(row)+"\\n")
os.execve({str(binary)!r},[{str(binary)!r},*args],env)
"""
    path.write_text(code)
    path.chmod(0o700)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--go", type=Path, required=True)
    parser.add_argument("--archive", type=Path, required=True)
    args = parser.parse_args()
    if platform.system() != "Darwin":
        parser.error("Pinned sandbox grading requires macOS.")
    binary = shutil.which("claude")
    if not binary:
        parser.error("Installed, authenticated Claude Code required.")
    plan_path = ROOT / "bench/repository/cron-pause/claude-campaign.json"
    plan = json.loads(plan_path.read_text())
    expected = {(e, r) for e in plan["entrants"] for r in range(1, plan["repeats"] + 1)}
    cells = [(c["entrant"], c["repeat"]) for c in plan["order"]]
    if (
        len(cells) != len(set(cells))
        or set(cells) != expected
        or set(plan["entrants"]) != {"claude", "bruhs-claude"}
    ):
        raise ValueError("Every declared pair must appear once.")
    timeout = float(plan["timeout_seconds_per_provider_and_grading_step"])
    campaign = Campaign(server_path(args.server), args.output.resolve(), timeout)
    report_path = campaign.output / "report.json"
    report = {
        "campaign": plan,
        "results": [],
        "preflight": None,
        "fatal_error": None,
        "billed_cost_usd": None,
        "human_time_seconds": None,
        "server_sha256": digest(campaign.server),
        "claude_version": subprocess.check_output(
            [binary, "--version"], text=True
        ).strip(),
        "claude_sha256": digest(Path(binary).resolve()),
        "recorded_at": datetime.now(UTC).isoformat(),
    }
    fixture = campaign.output / "prepared-fixture"
    prepare(
        ROOT / "bench/repository/cron-pause",
        args.archive.resolve(),
        fixture,
        args.go.resolve(),
    )
    save(fixture / "campaign.json", plan)
    go_wrapper(args.go.resolve(), campaign.home / "bin/go")
    go_file = campaign.home / "bin/go"
    go_file.write_text(
        go_file.read_text().replace("#!" + sys.executable, "#!/usr/bin/python3", 1)
    )
    wrapper(
        Path(binary).resolve(),
        campaign.home / "bin/claude",
        campaign.output / "provider-calls.jsonl",
        Path.home(),
        ROOT,
        campaign.output,
        plan["model"],
    )
    campaign.env.pop("BRUHS_FIXTURE_LOG", None)
    campaign.env.update(
        BRUHS_MAIL_DB=str(campaign.home / "mail.sqlite3"),
        BRUHS_REACTIVE_DB=str(campaign.home / "reactive.sqlite3"),
    )
    snapshot_root = campaign.output / "snapshot"

    def grade(frozen: Path, work: Path, output: Path, budget: float) -> int:
        command = [
            sys.executable,
            "-I",
            "-S",
            str(snapshot_root / "bench/repository/grade_cli.py"),
            "--fixture",
            str(frozen),
            "--work",
            str(work),
            "--output",
            str(output),
            "--timeout",
            str(budget),
        ]
        with Path(str(output) + ".driver.log").open("xb") as log:
            process = subprocess.Popen(
                command,
                stdin=subprocess.DEVNULL,
                stdout=log,
                stderr=subprocess.STDOUT,
                start_new_session=True,
                env={
                    "PATH": "/usr/bin:/bin",
                    "HOME": str(Path.home()),
                    "PYTHONDONTWRITEBYTECODE": "1",
                },
            )
            try:
                status = process.wait(timeout=budget)
            except subprocess.TimeoutExpired:
                stop_process_group(process)
                status = 124
        if not (output / "report.json").is_file():
            raise RuntimeError("Frozen grader produced no complete receipt.")
        return status

    def usage_metadata(log: Path) -> dict:
        rows = read_jsonl(str(log))
        events = [r for r in rows if r.get("envelope") in [None, "query"]]
        init = next(
            (
                r
                for r in events
                if r.get("type") == "init"
                or r.get("type") == "system"
                and r.get("subtype") == "init"
            ),
            {},
        )
        terminal = next((r for r in reversed(events) if r.get("type") == "result"), {})
        return {
            "reported_model": init.get("model"),
            "terminal_subtype": terminal.get("subtype"),
            "reported_model_cost_usd": terminal.get(
                "total_cost_usd", terminal.get("cost_usd")
            ),
            "billed_cost_usd": None,
            "mcp_server_names": [s.get("name") for s in init.get("mcp_servers", [])],
            "native_permission_denials": len(terminal.get("permission_denials", [])),
        }

    try:
        campaign.start()
        preflight = {
            "prompt": "Owned configuration preflight only. Do not call tools. Reply exactly PREFLIGHT_OK.",
            "project_path": str(campaign.project),
            "thread_id": "claude-cohort-preflight",
            "model": plan["model"],
            "effort": "high",
            "permission_mode": "acceptEdits",
            "fresh_session": True,
            "thread_access": "off",
            "execution_timeout_seconds": 90,
        }
        command = [
            "/usr/bin/curl",
            "--silent",
            "--show-error",
            "--no-buffer",
            "--fail-with-body",
            "-H",
            "Content-Type: application/json",
            "--data-binary",
            json.dumps(preflight),
            f"http://127.0.0.1:{campaign.port}/api/agent",
        ]
        with (
            (campaign.output / "host-preflight.log").open("xb") as out,
            (campaign.output / "host-preflight.err").open("xb") as err,
        ):
            preflight_result = subprocess.run(
                command,
                env=campaign.env,
                cwd=campaign.home,
                stdin=subprocess.DEVNULL,
                stdout=out,
                stderr=err,
                timeout=100,
                check=False,
            )
        meta = usage_metadata(campaign.output / "host-preflight.log")
        report["preflight"] = {"exit_code": preflight_result.returncode, **meta}
        save(report_path, report)
        rows = read_jsonl(str(campaign.output / "host-preflight.log"))
        done = [
            r
            for r in rows
            if r.get("type") == "result" and r.get("envelope") == "query"
        ]
        if (
            preflight_result.returncode
            or not done
            or done[-1].get("is_error") is not False
            or meta["reported_model"] != plan["model"]
        ):
            raise RuntimeError(
                "Host configuration preflight failed; no coding cells started."
            )
        frozen = freeze(campaign, fixture)
        report["frozen_inputs_sha256"] = frozen
        report["freeze_at"] = datetime.now(UTC).isoformat()
        save(report_path, report)
        for cell in plan["order"]:
            report["active_attempt"] = cell
            save(report_path, report)
            print(
                "START " + cell["entrant"] + " / " + cell["attempt_label"], flush=True
            )
            result = attempt(
                campaign,
                cell["entrant"],
                plan["task_id"],
                plan["model"],
                timeout,
                snapshot_root,
                attempt_label=cell["attempt_label"],
                grader=grade,
                deny_permissions=cell["entrant"] == "bruhs-claude",
            )
            result.update(
                repeat=cell["repeat"],
                attempt_label=cell["attempt_label"],
                reported_usage=usage_metadata(Path(result["log"])),
            )
            result["outcome"] = (
                "timeout"
                if result["exit"] == 124
                else "automatic_pass"
                if result["passed"]
                else "failed"
            )
            report["results"].append(result)
            report["active_attempt"] = None
            save(report_path, report)
            print(
                json.dumps(
                    {
                        k: result[k]
                        for k in [
                            "entrant",
                            "repeat",
                            "exit",
                            "completion",
                            "check_exit",
                            "outcome",
                            "reported_tokens",
                        ]
                    }
                ),
                flush=True,
            )
            require_actual_model(
                result["reported_usage"]["reported_model"], plan["model"]
            )
        report["frozen_inputs_unchanged"] = all(
            digest(snapshot_root / name) == value for name, value in frozen.items()
        )
        report.update(
            audit_binaries(
                campaign.server,
                Path(binary).resolve(),
                report["server_sha256"],
                report["claude_sha256"],
            )
        )
    except BaseException as error:  # noqa: BLE001 - record cleanup failure and fail the campaign
        report["fatal_error"] = {
            "type": type(error).__name__,
            "message": str(error),
            "traceback": traceback.format_exc(),
        }
        print(json.dumps(report["fatal_error"]), flush=True)
    finally:
        try:
            campaign.close()
        except BaseException as error:  # noqa: BLE001 - record cleanup failure and fail the campaign
            report["cleanup_error"] = str(error)
        report["active_attempt"] = None
        started = {(r["entrant"], r["repeat"]) for r in report["results"]}
        for cell in plan["order"]:
            if (cell["entrant"], cell["repeat"]) not in started:
                report["results"].append(
                    {
                        **cell,
                        "outcome": "unstarted",
                        "passed": False,
                        "monetary_cost": None,
                    }
                )
        save(report_path, report)
    return (
        0
        if not report["fatal_error"]
        and not report.get("cleanup_error")
        and len(started) == len(plan["order"])
        and report.get("frozen_inputs_unchanged") is True
        and report.get("provider_binary_unchanged") is True
        and report.get("server_binary_unchanged") is True
        else 1
    )


if __name__ == "__main__":
    raise SystemExit(main())
