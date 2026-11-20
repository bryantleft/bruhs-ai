"""Run the declared matched repository campaign with private application data."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import re
import shutil
import subprocess
import sys
import tempfile
import traceback
from pathlib import Path
from typing import TypedDict

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from bench.lib.run import stop_process_group  # noqa: E402
from bench.live.run import attempt, provider_wrapper  # noqa: E402
from bench.workbench.run import Campaign, digest  # noqa: E402


class RuntimeContext(TypedDict):
    model: object
    effort: object
    approval_policy: object
    sandbox_policy: object


class RuntimeSession(TypedDict):
    rollout: str
    initial_messages: list[dict[str, object]]
    skill_catalogs: list[str]
    turn_contexts: list[RuntimeContext]


def freeze(campaign: Campaign, fixture: Path) -> dict[str, str]:
    snapshot = campaign.output / "snapshot"
    declaration = fixture / "campaign.json"
    task = json.loads(declaration.read_text()).get("task_id", "huey-leases") if declaration.is_file() else "huey-leases"
    if not re.fullmatch(r"[a-z0-9-]+", task):
        raise ValueError("Repository task ID must be a safe directory name.")
    shutil.copytree(
        fixture,
        snapshot / "bench/tasks" / task,
        ignore=shutil.ignore_patterns("__pycache__", "*.pyc"),
    )
    files = [ROOT / "Cargo.lock"]
    for relative in ("bench/lib", "bench/live", "bench/repository", "bench/workbench"):
        files.extend((ROOT / relative).rglob("*.py"))
    for crate in (
        "bruhs-core",
        "bruhs-models",
        "bruhs-server",
        "codex-sdk-rs",
        "claude-sdk-rs",
    ):
        for file in (ROOT / crate).rglob("*"):
            if (
                file.is_file()
                and "target" not in file.parts
                and file.suffix in (".rs", ".toml", ".html", ".md")
            ):
                files.append(file)
    for file in files:
        if "__pycache__" in file.parts:
            continue
        if file.is_symlink():
            raise ValueError("Campaign source inputs must be regular files.")
        destination = snapshot / file.relative_to(ROOT)
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(file, destination)
    return {
        str(file.relative_to(snapshot)): digest(file)
        for file in sorted(snapshot.rglob("*"))
        if file.is_file()
    }


def validate_campaign(plan: dict) -> None:
    task = plan.get("task_id", "huey-leases")
    if not isinstance(task, str) or not re.fullmatch(r"[a-z0-9-]+", task):
        raise ValueError("Repository task ID must be a safe directory name.")
    if plan.get("language", "python") not in ("python", "go"):
        raise ValueError("Repository grading supports Python or Go fixtures.")
    timeout = plan["timeout_seconds_per_provider_and_grading_step"]
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("Campaign requires a positive finite declared timeout.")
    if type(plan["repeats"]) is not int or plan["repeats"] <= 0:
        raise ValueError("Campaign repeats must be a positive integer.")
    expected = {
        (entrant, repeat)
        for entrant in plan["entrants"]
        for repeat in range(1, plan["repeats"] + 1)
    }
    actual = [(cell["entrant"], cell["repeat"]) for cell in plan["order"]]
    if len(actual) != len(set(actual)) or set(actual) != expected:
        raise ValueError(
            "Campaign order must include every declared entrant/repeat exactly once."
        )
    if set(plan["entrants"]) != {"codex", "bruhs-codex"}:
        raise ValueError(
            "This campaign supports only installed Codex and the Bruhs Codex path."
        )
    if any(
        not re.fullmatch(r"[a-z0-9-]+", cell["attempt_label"]) for cell in plan["order"]
    ):
        raise ValueError("Attempt labels must be distinct safe directory names.")
    labels = [(cell["entrant"], cell["attempt_label"]) for cell in plan["order"]]
    if len(labels) != len(set(labels)):
        raise ValueError("One entrant cannot reuse an attempt directory label.")
    settings = plan.get("provider_config", {})
    if not isinstance(settings, dict) or any(
        not isinstance(key, str) or type(value) is not bool
        for key, value in settings.items()
    ):
        raise ValueError(
            "Provider config overrides must be a mapping of names to booleans."
        )
    if type(plan.get("native_cli_no_daemon", False)) is not bool:
        raise ValueError("Native daemon policy must be explicit boolean configuration.")


def audit_runtime(output: Path, work: Path, model: str | None = None, effort: str | None = None) -> dict:
    """Retain actual initial context fingerprints and reject injected skill catalogs."""
    matches: list[RuntimeSession] = []
    for path in sorted((output / "sessions").rglob("*.jsonl")):
        rows = [json.loads(line) for line in path.read_text().splitlines()]
        if not any(
            row.get("type") == "session_meta"
            and row.get("payload", {}).get("cwd") == str(work)
            for row in rows
        ):
            continue
        initial: list[dict[str, object]] = []
        catalogs = []
        contexts: list[RuntimeContext] = []
        before_assistant = True
        for row in rows:
            payload = row.get("payload", {})
            if row.get("type") == "turn_context":
                contexts.append({"model": payload.get("model"), "effort": payload.get("effort"),
                                 "approval_policy": payload.get("approval_policy"), "sandbox_policy": payload.get("sandbox_policy")})
            if row.get("type") != "response_item" or payload.get("type") != "message":
                continue
            role = payload.get("role")
            value = "".join(
                part.get("text", "")
                for part in payload.get("content", [])
                if isinstance(part, dict)
            )
            if role == "assistant":
                before_assistant = False
            if role == "developer" and value.lstrip().startswith(
                "<skills_instructions>"
            ):
                catalogs.append(hashlib.sha256(value.encode()).hexdigest())
            if before_assistant:
                initial.append(
                    {
                        "role": role,
                        "characters": len(value),
                        "sha256": hashlib.sha256(value.encode()).hexdigest(),
                    }
                )
        matches.append(
            {
                "rollout": str(path),
                "initial_messages": initial,
                "skill_catalogs": catalogs,
                "turn_contexts": contexts,
            }
        )
    settings_match = True
    if model is not None or effort is not None:
        contexts = [context for session in matches for context in session["turn_contexts"]]
        settings_match = bool(contexts) and all(
            isinstance(context["sandbox_policy"], dict)
            and (model is None or context["model"] == model)
            and (effort is None or context["effort"] == effort)
            and context["approval_policy"] == "never"
            and context["sandbox_policy"].get("type") == "workspace-write"
            and context["sandbox_policy"].get("network_access") is False
            for context in contexts
        )
    return {
        "passed": len(matches) == 1 and not matches[0]["skill_catalogs"] and settings_match,
        "sessions": matches,
        "requirement": "One fresh rollout, no automatic skills catalog and any declared model/effort with workspace-write, never approval and no command network.",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=Path, required=True)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if platform.system() != "Darwin":
        parser.error("Repository grading requires the macOS Seatbelt runner.")
    codex = shutil.which("codex")
    auth = Path(os.environ.get("CODEX_HOME", Path.home() / ".codex")) / "auth.json"
    if codex is None or not auth.is_file():
        parser.error("Installed Codex and existing sign-in are required.")
    plan = json.loads((args.fixture / "campaign.json").read_text())
    validate_campaign(plan)
    timeout = float(plan["timeout_seconds_per_provider_and_grading_step"])
    campaign = Campaign(args.server.resolve(), args.output.resolve(), timeout)
    frozen = freeze(campaign, args.fixture.resolve())
    snapshot = campaign.output / "snapshot"
    report_path = campaign.output / "report.json"
    version = subprocess.run(
        [codex, "--version"],
        capture_output=True,
        text=True,
        check=True,
        timeout=timeout,
    ).stdout.strip()
    report = {
        "campaign": plan,
        "frozen_inputs_sha256": frozen,
        "server_sha256": digest(campaign.server),
        "codex_version": version,
        "codex_sha256": digest(Path(codex).resolve()),
        "results": [],
        "fatal_error": None,
        "cleanup_error": None,
        "manual_review": "pending; automatic results are provisional until diff review",
        "billed_cost": None,
    }
    if plan.get("language") == "go":
        from bench.repository.go_fixture import provider_wrapper as go_wrapper

        inventory = json.loads((args.fixture / "inventory.json").read_text())
        go = Path(inventory["go_binary"])
        if digest(go) != inventory["go_sha256"]:
            raise ValueError("The Go compiler changed after fixture preparation.")
        go_wrapper(go, campaign.home / "bin/go")
        report["go_version"] = subprocess.run(
            [str(campaign.home / "bin/go"), "version"], cwd=campaign.home, env=campaign.env,
            capture_output=True, text=True, check=True, timeout=timeout,
        ).stdout.strip()
        report["go_sha256"] = digest(go)
    report_path.write_text(json.dumps(report, indent=2) + "\n")

    def frozen_grade(fixture: Path, work: Path, output: Path, budget: float) -> int:
        command = [
            sys.executable,
            "-I",
            "-S",
            str(snapshot / "bench/repository/grade_cli.py"),
            "--fixture",
            str(fixture),
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
                # The profile must deny the actual host home,
                # not only the synthetic provider HOME.
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
            except BaseException:
                stop_process_group(process)
                raise
        if not (output / "report.json").is_file():
            raise RuntimeError(
                "Repository grader did not produce a complete report; inspect its driver log."
            )
        return status

    previous_codex_home = os.environ.get("CODEX_HOME")
    try:
        with tempfile.TemporaryDirectory(prefix="bruhs-repository-auth-") as temporary:
            provider_home = Path(temporary)
            (provider_home / "auth.json").symlink_to(auth.resolve())
            sessions = campaign.output / "sessions"
            sessions.mkdir()
            (provider_home / "sessions").symlink_to(sessions)
            campaign.env["CODEX_HOME"] = str(provider_home)
            campaign.env["BRUHS_MAIL_DB"] = str(campaign.home / "mail.sqlite3")
            campaign.env["BRUHS_REACTIVE_DB"] = str(campaign.home / "reactive.sqlite3")
            campaign.env.pop("BRUHS_FIXTURE_LOG", None)
            provider_wrapper(
                Path(codex).resolve(),
                campaign.home / "bin/codex",
                campaign.output / "provider-calls.jsonl",
                plan["effort"],
                config_overrides=plan.get("provider_config"),
                no_daemon=plan.get("native_cli_no_daemon", False),
            )
            os.environ["CODEX_HOME"] = str(provider_home)
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
                timeout=timeout,
            )
            campaign.start()
            for cell in plan["order"]:
                report["active_attempt"] = cell
                report_path.write_text(json.dumps(report, indent=2) + "\n")
                print(f"START {cell['entrant']} / {cell['attempt_label']}", flush=True)
                try:
                    result = attempt(
                        campaign,
                        cell["entrant"],
                        plan.get("task_id", "huey-leases"),
                        plan["model"],
                        timeout,
                        snapshot,
                        attempt_label=cell["attempt_label"],
                        grader=frozen_grade,
                    )
                    result.update(
                        {
                            "repeat": cell["repeat"],
                            "attempt_label": cell["attempt_label"],
                            "outcome": "automatic_pass"
                            if result["passed"]
                            else "failed",
                        }
                    )
                    if result["exit"] == 124:
                        result["outcome"] = "timeout"
                    result["runtime_audit"] = audit_runtime(
                        campaign.output, Path(result["workdir"]), plan["model"], plan["effort"]
                    )
                    if not result["runtime_audit"]["passed"]:
                        result.update(
                            passed=False,
                            outcome="harness_error",
                            harness_error="Actual provider context failed the declared model, effort or sandbox/catalog policy.",
                        )
                except Exception:
                    result = {
                        **cell,
                        "passed": False,
                        "outcome": "harness_error",
                        "harness_error": traceback.format_exc(),
                    }
                report["results"].append(result)
                report["active_attempt"] = None
                report_path.write_text(json.dumps(report, indent=2) + "\n")
                print(
                    f"{result['outcome']} {cell['entrant']} / {cell['attempt_label']}",
                    flush=True,
                )
                if result.get("outcome") == "harness_error":
                    raise RuntimeError(
                        "Runtime context mismatch; campaign stopped with all cells retained."
                    )
            campaign.stop()
        report["temporary_auth_removed"] = not provider_home.exists()
    except (Exception, KeyboardInterrupt):
        report["fatal_error"] = traceback.format_exc()
    finally:
        if "provider_home" in locals():
            report["temporary_auth_removed"] = not provider_home.exists()
        if previous_codex_home is None:
            os.environ.pop("CODEX_HOME", None)
        else:
            os.environ["CODEX_HOME"] = previous_codex_home
        try:
            campaign.close()
        except Exception:
            report["cleanup_error"] = traceback.format_exc()
        done = {(row["entrant"], row["repeat"]) for row in report["results"]}
        report["results"].extend(
            {
                **cell,
                "passed": False,
                "outcome": "interrupted"
                if cell == report.get("active_attempt")
                else "not_run",
            }
            for cell in plan["order"]
            if (cell["entrant"], cell["repeat"]) not in done
        )
        report["frozen_inputs_unchanged"] = frozen == {
            str(file.relative_to(snapshot)): digest(file)
            for file in sorted(snapshot.rglob("*"))
            if file.is_file() and "__pycache__" not in file.parts
        }
        report["codex_binary_unchanged"] = (
            digest(Path(codex).resolve()) == report["codex_sha256"]
        )
        if plan.get("language") == "go":
            report["go_binary_unchanged"] = digest(go) == report["go_sha256"]
        report_path.write_text(json.dumps(report, indent=2) + "\n")
        print(f"Evidence: {report_path}", flush=True)
    return (
        0
        if (
            all(row["passed"] for row in report["results"])
            and report["frozen_inputs_unchanged"]
            and report["codex_binary_unchanged"]
            and report.get("go_binary_unchanged", True)
            and not report["fatal_error"]
            and not report["cleanup_error"]
        )
        else 1
    )


if __name__ == "__main__":
    sys.exit(main())
