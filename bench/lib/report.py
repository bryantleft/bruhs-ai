"""Render a bench results.tsv as markdown.

usage: report.py RESULTS_TSV [TITLE_LINE...]

Prints a per-run table, a per-tool summary (one row per tool and task kind:
coding tasks and chat tasks are summed separately, since review mode and the
file-editing harnesses only run coding tasks), and, when a cell ran more
than once (BENCH_RUNS), a per tool/task table. Extra arguments are printed
as lines under the heading (run settings). Reads results files from before
the kind/run columns existed too.
"""

from __future__ import annotations

import csv
import json
import math
from pathlib import Path
import sys


def num(value):
    try:
        result = int(value)
        return result if result >= 0 else None
    except (TypeError, ValueError):
        return None


def fmt(value):
    number = num(value)
    return f"{number:,}" if number is not None else (value or "-")


def k(number):
    return f"{number:,}" if number is not None else "-"


def spread(rows):
    """(mean, min, max, unknown count) of the known token counts."""
    tokens = [num(row["tokens"]) for row in rows]
    known = [t for t in tokens if t is not None]
    if not known:
        return None, None, None, len(tokens)
    return sum(known) // len(known), min(known), max(known), len(tokens) - len(known)


def ordered(values):
    seen = []
    for value in values:
        if value not in seen:
            seen.append(value)
    return seen


def passed(row: dict[str, str]) -> bool:
    return row["pass"] == "1" and row.get("exit") == "0" and row.get("completion", "completed") in ("completed", "process-only")


def outcome(row: dict[str, str]) -> str:
    if num(row.get("exit")) is None or (row.get("exit") == "0" and row.get("completion") == "unknown"):
        return "UNVERIFIED"
    return "PASS" if passed(row) else "FAIL"


def cell(row: dict[str, str]) -> tuple[str, str, str]:
    return row["kind"], row["task"], row["run"]


def wall_seconds(row: dict[str, str]) -> float | None:
    try:
        value = float(row["wall_s"])
    except (KeyError, ValueError, TypeError):
        return None
    return value if math.isfinite(value) and value >= 0 else None


def measured_total(values: list[int | float | None], missing: int = 0) -> str:
    known = [value for value in values if value is not None]
    unknown = len(values) - len(known) + missing
    if not known:
        return "-"
    total = f"{sum(known):,.0f}"
    return f">={total} ({unknown} unknown)" if unknown else total


def planned_cells(path: Path, rows: list[dict[str, str]]) -> tuple[list[str], set[tuple[str, str, str]]]:
    manifest_path = path.with_name("manifest.json")
    if not manifest_path.exists():
        return ordered(row["tool"] for row in rows), {cell(row) for row in rows}
    with manifest_path.open(encoding="utf-8") as handle:
        manifest = json.load(handle)
    tools = manifest["tools"]
    if any(row["tool"] not in tools for row in rows):
        raise ValueError("results contain an unplanned tool; compare against the original manifest")
    planned = {
        (task["kind"], task["name"], str(run))
        for task in manifest["tasks"]
        for run in range(1, manifest["runs"] + 1)
    }
    return tools, planned


def main():
    path = Path(sys.argv[1])
    with open(path, encoding="utf-8") as handle:
        rows = list(csv.DictReader(handle, delimiter="\t"))
    for row in rows:
        row.setdefault("kind", "code")
        row["kind"] = row.get("kind") or "code"
        row["run"] = row.get("run") or "1"
    repeated = any(row["run"] != "1" for row in rows)
    tools, planned = planned_cells(path, rows)
    identities = [(row["tool"], cell(row)) for row in rows]
    if len(set(identities)) != len(identities):
        raise ValueError("duplicate tool/task/kind/run rows; restore one record per attempt before comparing")
    if any(cell(row) not in planned for row in rows):
        raise ValueError("results contain an unplanned task or repeat; compare against the original manifest")

    out = ["# Task bench", ""]
    out += [f"- {line}" for line in sys.argv[2:]]
    out += ["", "## Runs", ""]
    cols = ["tool", "task", "kind", "run", "pass", "tokens", "peak_ctx", "turns", "tool_calls", "wall_s", "note"]
    out.append("| " + " | ".join(cols) + " |")
    out.append("|" + "|".join(["---"] * len(cols)) + "|")
    for row in rows:
        cells = [
            row["tool"],
            row["task"],
            row["kind"],
            row["run"],
            outcome(row),
            fmt(row["tokens"]),
            fmt(row["peak_ctx"]),
            fmt(row["turns"]),
            fmt(row["tool_calls"]),
            row["wall_s"],
            "; ".join(filter(None, [
                f"checks passed; exit {row['exit']}" if row.get("check_pass", row["pass"]) == "1"
                and row.get("exit") not in ("", "0", None) else "",
                row.get("note") or "",
            ])).replace("|", "/"),
        ]
        out.append("| " + " | ".join(cells) + " |")

    out += ["", "## Coverage", ""]
    out += [
        "Every selected task and repeat stays in the denominator. Missing or unsupported runs are incomplete coverage.",
        "Matching coverage alone does not establish matching models, settings, or environments.", "",
        "| tool | kind | recorded / planned | missing | status |", "|---|---|---|---|---|",
    ]
    for tool in tools:
        for kind in sorted({kind for kind, _, _ in planned}):
            expected = {item for item in planned if item[0] == kind}
            recorded = {cell(row) for row in rows if row["tool"] == tool and row["kind"] == kind}
            missing = expected - recorded
            out.append(f"| {tool} | {kind} | {len(recorded)}/{len(expected)} | {len(missing)} | "
                       f"{'incomplete' if missing else 'complete'} |")

    out += ["", "## Per tool", ""]
    out.append(
        "| tool | kind | passed | tokens mean / run | min | max | tokens (all runs) "
        "| tokens / passed run | peak ctx (max) | wall s (sum) |"
    )
    out.append("|---|---|---|---|---|---|---|---|---|---|")
    for tool in tools:
        for kind in sorted({kind for kind, _, _ in planned}):
            mine = [row for row in rows if row["tool"] == tool and row["kind"] == kind]
            expected = {item for item in planned if item[0] == kind}
            missing = len(expected) - len(mine)
            successes = sum(passed(row) for row in mine)
            mean, low, high, unknown = spread(mine)
            unknown += missing
            known = [num(row["tokens"]) for row in mine if num(row["tokens"]) is not None]
            total = sum(known) if known else None
            per_pass = f"{total // successes:,}" if total is not None and successes and not unknown else "-"
            peaks = [num(row["peak_ctx"]) for row in mine if num(row["peak_ctx"]) is not None]
            out.append(
                f"| {tool} | {kind} | {successes}/{len(expected)} | {k(mean if not unknown else None)} | {k(low)} | {k(high)} | "
                f"{measured_total([num(row['tokens']) for row in mine], missing)} | {per_pass} | "
                f"{k(max(peaks) if peaks else None)} | {measured_total([wall_seconds(row) for row in mine], missing)} |"
            )

    if repeated:
        out += ["", "## Per tool and task", ""]
        out.append("| tool | task | passed | tokens mean | min | max |")
        out.append("|---|---|---|---|---|---|")
        for tool in ordered(row["tool"] for row in rows):
            for task in ordered(row["task"] for row in rows if row["tool"] == tool):
                mine = [row for row in rows if row["tool"] == tool and row["task"] == task]
                successes = sum(passed(row) for row in mine)
                mean, low, high, unknown = spread(mine)
                expected = {item for item in planned if item[1] == task}
                complete = len(expected) == len(mine) and not unknown
                out.append(f"| {tool} | {task} | {successes}/{len(expected)} | {k(mean if complete else None)} | {k(low)} | {k(high)} |")
    print("\n".join(out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
