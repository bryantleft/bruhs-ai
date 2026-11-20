"""Freeze benchmark fixtures and record the selected comparison before runs start."""

from __future__ import annotations

import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys


def snapshot(root: Path, output: Path, tasks: list[str], tools: list[str], runs: int,
             settings: dict[str, str]) -> None:
    if not tasks or not tools or len(set(tasks)) != len(tasks) or len(set(tools)) != len(tools):
        raise ValueError("select nonempty task and tool lists without duplicates")
    timeout = float(settings["timeout_seconds_per_step"])
    if runs < 1 or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("runs and per-step timeout must be positive and finite")
    destination = output / "snapshot"
    files: dict[str, str] = {}
    sources = [root / "bench/lib", *(root / "bench/tasks" / task for task in tasks)]
    for source in sources:
        for path in sorted(source.rglob("*")):
            if "__pycache__" in path.parts or path.name == ".DS_Store":
                continue
            if path.is_symlink():
                raise ValueError(f"benchmark input is a symlink: {path}; use a regular fixture file")
            if path.is_file():
                relative = path.relative_to(root)
                target = destination / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                data = path.read_bytes()
                target.write_bytes(data)
                shutil.copymode(path, target)
                files[relative.as_posix()] = hashlib.sha256(data).hexdigest()
    runner = root / "scripts/task-bench.sh"
    relative_runner = runner.relative_to(root)
    (destination / relative_runner).parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(runner, destination / relative_runner)
    files[relative_runner.as_posix()] = hashlib.sha256((destination / relative_runner).read_bytes()).hexdigest()
    revision = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"],
                              capture_output=True, text=True, check=False)
    kinds = []
    for task in tasks:
        kind_path = destination / "bench/tasks" / task / "kind"
        kind = "".join(kind_path.read_text(encoding="utf-8").split()) if kind_path.exists() else "code"
        if kind not in ("code", "chat"):
            raise ValueError(f"unsupported task kind {kind!r} in {task}")
        kinds.append({"name": task, "kind": kind})
    manifest = {
        "schema_version": 1,
        "tools": tools,
        "tasks": kinds,
        "runs": runs,
        "settings": settings,
        "extra_cli_args_sha256": {
            name: hashlib.sha256(os.environ.get(name, "").encode()).hexdigest()
            for name in ("BENCH_CLAUDE_ARGS", "BENCH_CODEX_ARGS")
        },
        "source_revision": revision.stdout.strip() if revision.returncode == 0 else None,
        "platform": {"system": platform.system(), "machine": platform.machine(), "python": platform.python_version()},
        "files_sha256": files,
        "snapshot_sha256": hashlib.sha256(json.dumps(files, sort_keys=True).encode()).hexdigest(),
        "limits": ["Provider defaults, installed tool versions and user settings are not frozen; verify them before a model-controlled comparison.",
                   "Tokens measure reported volume, not total monetary cost or unreported descendant usage."],
    }
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


def main() -> None:
    root, output, tasks, tools, runs, timeout, claude, codex, bruhs, bruhs_codex = sys.argv[1:]
    snapshot(Path(root), Path(output), tasks.split(), tools.split(), int(runs), {
        "timeout_seconds_per_step": timeout,
        "claude_model": claude,
        "codex_model": codex or "configured default (not frozen)",
        "bruhs_model": bruhs,
        "bruhs_codex_model": bruhs_codex,
        "aider_model": os.environ.get("BENCH_AIDER_MODEL", "configured default (not frozen)"),
        "opencode_model": os.environ.get("BENCH_OPENCODE_MODEL", "configured default (not frozen)"),
    })


if __name__ == "__main__":
    main()
