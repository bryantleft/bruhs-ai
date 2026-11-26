"""Reparse saved logs without dropping grading evidence or TSV empty fields."""

from __future__ import annotations

import csv
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

from completion import completion


def rescore(directory: Path) -> None:
    results = directory / "results.tsv"
    with results.open(encoding="utf-8", newline="") as handle:
        reader = csv.DictReader(handle, delimiter="\t")
        rows = list(reader)
        fields = list(reader.fieldnames or [])
    legacy = "run" not in fields
    for field in ("kind", "run", "check_pass", "completion", "log_stem"):
        if field not in fields:
            fields.append(field)
    library = Path(__file__).resolve().parent
    for row in rows:
        row["kind"] = row.get("kind") or "code"
        row["run"] = row.get("run") or "1"
        row["check_pass"] = row.get("check_pass") or row["pass"]
        tool = row["tool"]
        fmt = {"bruhs": "bruhs-claude", "bruhs-review": "bruhs-review-claude",
               "bruhs-codex-review": "bruhs-review-codex"}.get(tool, tool)
        stem = row.get("log_stem") or f"{tool}-{row['task']}" + ("" if legacy else f"-{row['run']}")
        if Path(stem).name != stem:
            raise ValueError(f"invalid log filename stem {stem!r}")
        row["log_stem"] = stem
        log = str(directory / f"{stem}.log")
        output = subprocess.run([sys.executable, str(library / "metrics.py"), fmt, log],
                                capture_output=True, text=True, check=True)
        values = output.stdout.rstrip("\n").split("\t")
        if len(values) != 5:
            raise ValueError(f"metrics parser returned {len(values)} fields for {stem}, expected 5")
        row.update(zip(("tokens", "peak_ctx", "turns", "tool_calls", "note"), values))
        row["completion"] = completion(fmt, log)
        row["pass"] = str(int(row["check_pass"] == "1" and row.get("exit") == "0"
                              and row["completion"] in ("completed", "process-only")))
        if row.get("exit") != "0":
            row["note"] = f"exit {row.get('exit') or 'unknown'}; {row['note']}"
        if row["completion"] != "completed":
            row["note"] = f"completion {row['completion']}; {row['note']}"
    temporary = results.with_suffix(".tsv.new")
    with temporary.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=fields, delimiter="\t", lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)
    original = results.with_name("results.original.tsv")
    if not original.exists():
        shutil.copy2(results, original)
    temporary.replace(results)
    evidence = {
        "rescored_at": datetime.now(timezone.utc).isoformat(),
        "parser_files_sha256": {
            name: hashlib.sha256((library / name).read_bytes()).hexdigest()
            for name in ("metrics.py", "completion.py", "report.py", "rescore.py")
        },
    }
    (directory / "rescore.json").write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    with (directory / "meta.txt").open("a", encoding="utf-8") as handle:
        handle.write(f"rescored {evidence['rescored_at']}; original rows saved in results.original.tsv\n")


if __name__ == "__main__":
    rescore(Path(sys.argv[1]))
