"""Run a small, model-controlled Codex/Bruhs coding pilot on macOS.

Provider invocations are products under test, not implementation delegates.
Credentials stay outside retained artifacts; generated code runs in sandboxes.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import platform
import shutil
import subprocess
import sys
import tempfile
import time
import traceback
from collections.abc import Callable
from datetime import UTC, datetime
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# The shared CLI helpers retain sibling imports when used as scripts.
sys.path.insert(0, str(ROOT / "bench/lib"))
sys.path.insert(0, str(ROOT))
from bench.lib.completion import completion  # noqa: E402
from bench.lib.metrics import (  # noqa: E402 - sibling CLI helper path
    bruhs_claude_metrics,
    bruhs_codex_metrics,
    claude_metrics,
    codex_metrics,
    read_jsonl,
)
from bench.lib.provenance import snapshot  # noqa: E402
from bench.lib.run import stop_process_group  # noqa: E402
from bench.live.grading import grade, result_path  # noqa: E402
from bench.live.permissions import PermissionDenier, communicate_declining  # noqa: E402
from bench.workbench.run import Campaign, digest  # noqa: E402

TASKS = ("rootcause-paging", "concurrency-keyed-jobs")
ENTRANTS = ("codex", "bruhs-codex")


def provider_wrapper(
    binary: Path,
    path: Path,
    calls: Path,
    effort: str,
    config_overrides: dict[str, bool] | None = None,
    no_daemon: bool = False,
) -> None:
    """Apply identical native sandbox/configuration constraints to both paths."""
    settings = config_overrides or {}
    extra = [
        f"--config={key}={json.dumps(value)}" for key, value in sorted(settings.items())
    ]
    if no_daemon:
        extra.append("--no-daemon")
    source = f"""#!{sys.executable}
import json, os, sys
args = sys.argv[1:]
if "exec" in args:
    if any("dangerously" in arg or "danger-full-access" in arg for arg in args):
        raise SystemExit("Live pilot refuses unsandboxed execution.")
    def option(name):
        for index, arg in enumerate(args):
            if arg.startswith(name + "="):
                return arg.split("=", 1)[1]
            if arg == name and index + 1 < len(args):
                return args[index + 1]
        return None
    if option("--sandbox") != "workspace-write" or option("--ask-for-approval") != "never":
        raise SystemExit("Live pilot requires explicit workspace-write sandbox and never approval policy.")
    index = args.index("exec") + 1
    args[index:index] = ["--ignore-user-config", "--ignore-rules"]
    args[:0] = {extra!r} + ['--config=model_reasoning_effort={json.dumps(effort)}',
                '--config=sandbox_workspace_write.network_access=false']
    # Record only non-secret invocation evidence. MCP config can contain tokens.
    with open({str(calls)!r}, "a", encoding="utf-8") as log:
        log.write(json.dumps({{"cwd": os.getcwd(), "native_sandbox_required": True,
                              "ignore_user_config": True, "ignore_rules": True,
                              "effort": {effort!r}, "config_overrides": {settings!r},
                              "no_daemon": {no_daemon!r}}}) + "\\n")
os.execv({str(binary)!r}, [{str(binary)!r}, *args])
"""
    path.write_text(source, encoding="utf-8")
    path.chmod(0o700)


def capture_patch(work: Path, baseline: str, stem: Path, env: dict[str, str]) -> bytes:
    """Include new files and committed work without changing the submitted index."""
    work, stem = work.resolve(), stem.resolve()
    index = stem.with_suffix(".index")
    if index.exists():
        raise ValueError("Patch capture requires a fresh index artifact path.")
    diff_env = dict(env, GIT_INDEX_FILE=str(index))
    subprocess.run(
        ["/usr/bin/git", "-C", str(work), "read-tree", baseline],
        capture_output=True,
        check=True,
        env=diff_env,
    )
    subprocess.run(
        ["/usr/bin/git", "-C", str(work), "add", "-A"],
        capture_output=True,
        check=True,
        env=diff_env,
    )
    return subprocess.run(
        ["/usr/bin/git", "-C", str(work), "diff", "--cached", "--binary", baseline],
        capture_output=True,
        check=True,
        env=diff_env,
    ).stdout


def attempt(
    campaign: Campaign,
    entrant: str,
    task: str,
    model: str,
    timeout: float,
    snapshot_root: Path,
    attempt_label: str | None = None,
    grader: Callable[[Path, Path, Path, float], int] = grade,
    deny_permissions: bool = False,
) -> dict:
    if deny_permissions and entrant != "bruhs-claude":
        raise ValueError(
            "Non-interactive permission denials are declared only for the Bruhs Claude cohort."
        )
    label = attempt_label or task
    work = campaign.home / "attempts" / entrant / label
    fixture = snapshot_root / "bench/tasks" / task
    work.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(fixture / "repo", work)
    for args in (
        ["init", "-b", "main"],
        ["add", "."],
        [
            "-c",
            "user.name=pilot",
            "-c",
            "user.email=pilot@localhost",
            "commit",
            "-m",
            "baseline",
        ],
    ):
        subprocess.run(
            ["/usr/bin/git", "-C", str(work), *args],
            env=campaign.env,
            check=True,
            capture_output=True,
        )
    baseline = subprocess.run(
        ["/usr/bin/git", "-C", str(work), "rev-parse", "HEAD"],
        env=campaign.env,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    stem = campaign.output / f"{entrant}-{label}"
    prompt = (fixture / "prompt.md").read_text(encoding="utf-8")
    if entrant == "codex":
        command = [
            str(campaign.home / "bin/codex"),
            "--ask-for-approval=never",
            "exec",
            "--json",
            "--sandbox=workspace-write",
            f"--model={model}",
            f"--cd={work}",
            "-",
        ]
        input_data = prompt.encode()
    elif entrant == "claude":
        command = [
            str(campaign.home / "bin/claude"),
            "-p",
            "--verbose",
            "--output-format=stream-json",
            "--permission-mode=acceptEdits",
            f"--model={model}",
        ]
        input_data = prompt.encode()
    else:
        body = {
            "prompt": prompt,
            "project_path": str(work),
            "thread_id": f"pilot-{label}",
            "model": model if entrant == "bruhs-claude" else f"codex/{model}",
            "permission_mode": "acceptEdits",
            "thread_access": "off",
            "fresh_session": True,
        }
        body_path = stem.with_suffix(".body.json")
        body_path.write_text(json.dumps(body), encoding="utf-8")
        command = [
            "/usr/bin/curl",
            "--silent",
            "--show-error",
            "--no-buffer",
            "--fail-with-body",
            "-H",
            "Content-Type: application/json",
            "--data-binary",
            f"@{body_path}",
            f"http://127.0.0.1:{campaign.port}/api/agent",
        ]
        input_data = b""
    started = time.monotonic()
    with (
        stem.with_suffix(".log").open("wb") as out,
        stem.with_suffix(".err").open("wb") as err,
    ):
        process = subprocess.Popen(
            command,
            cwd=work,
            env=campaign.env,
            stdin=subprocess.PIPE,
            stdout=out,
            stderr=err,
            start_new_session=True,
        )
        try:
            if deny_permissions:
                communicate_declining(
                    process,
                    input_data,
                    timeout,
                    PermissionDenier(
                        campaign.port,
                        stem.with_suffix(".log"),
                        stem.with_suffix(".permission-decisions.jsonl"),
                    ),
                )
            else:
                process.communicate(input=input_data, timeout=timeout)
            status = process.returncode
        except subprocess.TimeoutExpired:
            stop_process_group(process)
            status = 124
            # The HTTP client is not the provider's process owner. Stop the owned
            # server before grading a timed-out turn; never grade a moving tree.
            if entrant in {"bruhs-codex", "bruhs-claude"}:
                campaign.stop()
                campaign.start()
        except BaseException:
            stop_process_group(process)
            raise
    seconds = time.monotonic() - started
    log = str(stem.with_suffix(".log"))
    terminal = completion(entrant, log)
    rows = read_jsonl(log)
    parsers = {
        "codex": codex_metrics,
        "bruhs-codex": bruhs_codex_metrics,
        "claude": claude_metrics,
        "bruhs-claude": bruhs_claude_metrics,
    }
    metrics = parsers[entrant](rows)
    grading_started = time.monotonic()
    check_output = stem.with_suffix(".check")
    check_status = grader(fixture, work, check_output, timeout)
    grading_seconds = time.monotonic() - grading_started
    diff = capture_patch(work, baseline, stem, campaign.env)
    stem.with_suffix(".diff").write_bytes(diff)
    return {
        "entrant": entrant,
        "task": task,
        "exit": status,
        "completion": terminal,
        "check_exit": check_status,
        "passed": status == 0 and terminal == "completed" and check_status == 0,
        "check_result": str(
            check_output / "report.json"
            if check_output.is_dir()
            else result_path(check_output)
        ),
        "grading_seconds": grading_seconds,
        "wall_seconds": seconds,
        "reported_tokens": metrics[0],
        "peak_context": metrics[1],
        "model_requests": metrics[2],
        "tool_calls": metrics[3],
        "usage_note": metrics[4],
        "monetary_cost": None,
        "corrective_interventions": 0,
        "starting_commit": baseline,
        "log": str(stem.with_suffix(".log")),
        "workdir": str(work),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--model", required=True)
    parser.add_argument(
        "--effort", choices=("low", "medium", "high", "xhigh"), required=True
    )
    # Budget is declared by the caller before attempts, rather than a tuned timeout.
    parser.add_argument("--timeout", type=float, required=True)
    args = parser.parse_args()
    if platform.system() != "Darwin" or not Path("/usr/bin/sandbox-exec").exists():
        parser.error(
            "This pilot requires macOS Seatbelt; use another isolated runner on other platforms."
        )
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("--timeout must be positive and finite")
    codex = shutil.which("codex")
    auth = Path(os.environ.get("CODEX_HOME", Path.home() / ".codex")) / "auth.json"
    if codex is None or not auth.is_file():
        parser.error(
            "Install Codex and authenticate with codex login before this pilot."
        )
    campaign = Campaign(args.server.resolve(), args.output.resolve(), args.timeout)
    snapshot(
        ROOT,
        campaign.output,
        list(TASKS),
        list(ENTRANTS),
        1,
        {
            "timeout_seconds_per_step": str(args.timeout),
            "codex_model": args.model,
            "bruhs_codex_model": f"codex/{args.model}",
            "reasoning_effort": args.effort,
            "configuration": "Private CODEX_HOME; ignore user config/rules; native workspace-write, network disabled for generated commands.",
        },
    )
    driver_files = [
        Path(__file__),
        ROOT / "bench/live/grading.py",
        ROOT / "bench/lib/test_receipts.py",
        ROOT / "bench/lib/test_runner.py",
        ROOT / "bench/workbench/run.py",
        ROOT / "bench/workbench/fixture_codex.py",
        ROOT / "bruhs-core/src/agent.rs",
        ROOT / "bruhs-core/src/coordination.rs",
        ROOT / "bruhs-server/src/routes/agent.rs",
        ROOT / "codex-sdk-rs/src/query.rs",
        ROOT / "codex-sdk-rs/src/lib.rs",
        ROOT / "claude-sdk-rs/src/process.rs",
        ROOT / "Cargo.lock",
    ]
    for source in driver_files:
        dest = campaign.output / "snapshot" / source.relative_to(ROOT)
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, dest)
    sources = {str(path.relative_to(ROOT)): digest(path) for path in driver_files}
    version = subprocess.run(
        [codex, "--version"],
        capture_output=True,
        text=True,
        check=True,
        timeout=args.timeout,
    ).stdout.strip()
    report = {
        "scope": "Single-attempt diagnostic coding pilot; no competitor ranking or statistically supported win.",
        "started_at": datetime.now(UTC).isoformat(),
        "planned_attempts": [
            {"entrant": entrant, "task": task} for task in TASKS for entrant in ENTRANTS
        ],
        "model": args.model,
        "effort": args.effort,
        "timeout_seconds_per_attempt": args.timeout,
        "codex_version": version,
        "codex_sha256": digest(Path(codex).resolve()),
        "server_sha256": digest(campaign.server),
        "driver_sources_sha256": sources,
        "results": [],
        "fatal_error": None,
        "cleanup_error": None,
        "limits": [
            "Two internal fixtures, one attempt per entrant and task, fixed order; diagnostic only.",
            "Hidden tests and reference fixes are withheld from workspace/prompt; native Codex sandbox does not deny all host reads.",
            "Bruhs includes its normal response contract and app tools backed by private synthetic stores.",
            "Token volume is not money; subscription/compute/human costs are not measured.",
            "No Capy, Claude Code, T3 Code or connected daily-workflow execution in this pilot.",
        ],
    }
    report_path = campaign.output / "report.json"
    report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    try:
        # Only a credential symlink lives here. Neither it nor auth contents go
        # into retained artifacts; completed session logs remain for accounting.
        with tempfile.TemporaryDirectory(prefix="bruhs-live-auth-") as temporary:
            codex_home = Path(temporary)
            (codex_home / "auth.json").symlink_to(auth.resolve())
            sessions = campaign.output / "sessions"
            sessions.mkdir()
            (codex_home / "sessions").symlink_to(sessions)
            campaign.env["CODEX_HOME"] = str(codex_home)
            campaign.env["PATH"] += ":/opt/homebrew/bin"
            campaign.env["BRUHS_MAIL_DB"] = str(campaign.home / "mail.sqlite3")
            campaign.env["BRUHS_REACTIVE_DB"] = str(campaign.home / "reactive.sqlite3")
            campaign.env.pop("BRUHS_FIXTURE_LOG", None)
            provider_wrapper(
                Path(codex).resolve(),
                campaign.home / "bin/codex",
                campaign.output / "provider-calls.jsonl",
                args.effort,
            )
            # Metrics resolve the private, retained provider rollouts too.
            previous_home = os.environ.get("CODEX_HOME")
            os.environ["CODEX_HOME"] = str(codex_home)
            try:
                # Native command sandboxing must work before any model call.
                subprocess.run(
                    [
                        codex,
                        '--config=sandbox_mode="workspace-write"',
                        "--config=sandbox_workspace_write.network_access=false",
                        "sandbox",
                        "/usr/bin/true",
                    ],
                    cwd=campaign.home,
                    env=campaign.env,
                    check=True,
                    capture_output=True,
                    timeout=args.timeout,
                )
                campaign.start()
                for planned in report["planned_attempts"]:
                    report["active_attempt"] = planned
                    report_path.write_text(
                        json.dumps(report, indent=2) + "\n", encoding="utf-8"
                    )
                    print(f"START {planned['entrant']} / {planned['task']}", flush=True)
                    try:
                        result = attempt(
                            campaign,
                            planned["entrant"],
                            planned["task"],
                            model=args.model,
                            timeout=args.timeout,
                            snapshot_root=campaign.output / "snapshot",
                        )
                    except Exception:  # noqa: BLE001 - retain failures and all declared cells for diagnosis
                        result = {
                            **planned,
                            "passed": False,
                            "harness_error": traceback.format_exc(),
                        }
                    report["results"].append(result)
                    report["active_attempt"] = None
                    report_path.write_text(
                        json.dumps(report, indent=2) + "\n", encoding="utf-8"
                    )
                    print(
                        f"{'PASS' if result['passed'] else 'FAIL'} {planned['entrant']} / {planned['task']}",
                        flush=True,
                    )
            finally:
                campaign.stop()
                if previous_home is None:
                    os.environ.pop("CODEX_HOME", None)
                else:
                    os.environ["CODEX_HOME"] = previous_home
    except (Exception, KeyboardInterrupt):  # noqa: BLE001 - retain failed campaign and unstarted cells
        report["fatal_error"] = traceback.format_exc()
    finally:
        try:
            campaign.close()
        except Exception:  # noqa: BLE001 - retain failures and all declared cells for diagnosis
            report["cleanup_error"] = traceback.format_exc()
        attempted = {(row["entrant"], row["task"]) for row in report["results"]}
        report["results"].extend(
            {
                **planned,
                "passed": False,
                "not_run": (
                    "Campaign interrupted during this attempt."
                    if planned == report.get("active_attempt")
                    else "Campaign failed before this planned attempt."
                ),
            }
            for planned in report["planned_attempts"]
            if (planned["entrant"], planned["task"]) not in attempted
        )
        report["driver_sources_unchanged"] = sources == {
            str(path.relative_to(ROOT)): digest(path) for path in driver_files
        }
        report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(f"Evidence: {report_path}", flush=True)
    return (
        0
        if (
            len(report["results"]) == len(report["planned_attempts"])
            and all(result["passed"] for result in report["results"])
            and not report["fatal_error"]
            and not report["cleanup_error"]
            and report["driver_sources_unchanged"]
        )
        else 1
    )


if __name__ == "__main__":
    sys.exit(main())
