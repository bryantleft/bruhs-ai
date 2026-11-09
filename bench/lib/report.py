"""Render a bench results.tsv as markdown.

usage: report.py RESULTS_TSV [TITLE_LINE...]

Prints a per-run table, a per-tool summary (one row per tool and task kind:
coding tasks and chat tasks are summed separately, since review mode and the
file-editing harnesses only run coding tasks), and, when a cell ran more
than once (BENCH_RUNS), a per tool/task table. Extra arguments are printed
as lines under the heading (run settings). Reads results files from before
the kind/run columns existed too.
"""

import csv
import sys


def num(value):
    try:
        return int(value)
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


def main():
    path = sys.argv[1]
    with open(path, encoding="utf-8") as handle:
        rows = list(csv.DictReader(handle, delimiter="\t"))
    for row in rows:
        row.setdefault("kind", "code")
        row["kind"] = row.get("kind") or "code"
        row["run"] = row.get("run") or "1"
    repeated = any(row["run"] != "1" for row in rows)

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
            "PASS" if row["pass"] == "1" else "FAIL",
            fmt(row["tokens"]),
            fmt(row["peak_ctx"]),
            fmt(row["turns"]),
            fmt(row["tool_calls"]),
            row["wall_s"],
            (row.get("note") or "").replace("|", "/"),
        ]
        out.append("| " + " | ".join(cells) + " |")

    out += ["", "## Per tool", ""]
    out.append(
        "| tool | kind | passed | tokens mean / run | min | max | tokens (all runs) "
        "| tokens / passed run | peak ctx (max) | wall s (sum) |"
    )
    out.append("|---|---|---|---|---|---|---|---|---|---|")
    for tool in ordered(row["tool"] for row in rows):
        for kind in ordered(row["kind"] for row in rows if row["tool"] == tool):
            mine = [row for row in rows if row["tool"] == tool and row["kind"] == kind]
            passed = sum(row["pass"] == "1" for row in mine)
            mean, low, high, unknown = spread(mine)
            known = [num(row["tokens"]) for row in mine if num(row["tokens"]) is not None]
            total = sum(known) if known else None
            partial = f" ({unknown} unknown)" if known and unknown else ""
            per_pass = f"{total // passed:,}" if total is not None and passed else "-"
            peaks = [num(row["peak_ctx"]) for row in mine if num(row["peak_ctx"]) is not None]
            wall = sum(float(row["wall_s"] or 0) for row in mine if row["wall_s"] not in ("", "-"))
            out.append(
                f"| {tool} | {kind} | {passed}/{len(mine)} | {k(mean)} | {k(low)} | {k(high)} | "
                f"{k(total)}{partial} | {per_pass} | {k(max(peaks) if peaks else None)} | {wall:.0f} |"
            )

    if repeated:
        out += ["", "## Per tool and task", ""]
        out.append("| tool | task | passed | tokens mean | min | max |")
        out.append("|---|---|---|---|---|---|")
        for tool in ordered(row["tool"] for row in rows):
            for task in ordered(row["task"] for row in rows if row["tool"] == tool):
                mine = [row for row in rows if row["tool"] == tool and row["task"] == task]
                passed = sum(row["pass"] == "1" for row in mine)
                mean, low, high, _ = spread(mine)
                out.append(f"| {tool} | {task} | {passed}/{len(mine)} | {k(mean)} | {k(low)} | {k(high)} |")
    print("\n".join(out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
