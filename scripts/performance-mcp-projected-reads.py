#!/usr/bin/env python3
"""Measure projected native reads only against the private GPUI timing fixture."""
import argparse
import asyncio
import hashlib
import json
from pathlib import Path
import runpy
import time

ROOT = Path(__file__).resolve().parent.parent
TRANSPORT = ROOT / "scripts/performance-mcp-stdio.py"
helpers = runpy.run_path(str(TRANSPORT))
Client, frame, tool, summary = (helpers[name] for name in ("Client", "frame", "tool", "summary"))


def record_id(index, records):
    return f"record-{index}" + ("-evenslot" if index % 2 == 0 else "") + (
        "-lastquartz" if index == records - 1 else "")


def cases(records):
    return [
        ("first", "", 0, 0, 50, 1, 50),
        ("deep", "", records - 51, records - 51, 50, 1, records - 1),
        ("last", "LASTQUARTZ", 0, records - 1, 1, 1, None),
        ("none", "no-quartz-match", 0, 0, 0, 1, None),
        ("filtered-deep", "EVENSLOT", records // 2 - 50, records - 101, 50, 2, records // 2),
        ("filtered-final", "EVENSLOT", records // 2, records - 1, 1, 1, None),
        ("hidden-miss", "hidden-perf-body", 0, 0, 0, 1, None),
    ]


def check(reply, target, first, count, step, next_offset):
    assert reply["result"].get("isError") is not True, reply
    page = json.loads(reply["result"]["content"][0]["text"])
    assert len(page["records"]) == count
    assert page["next_offset"] == next_offset
    assert [record["id"] for record in page["records"]] == [
        record_id(first + index * step, target["records"]) for index in range(count)
    ]
    identity = target["identity"]
    if identity.startswith("finance/"):
        exact = identity.endswith("/exact")
        assert page["amounts_redacted"] is not exact
        assert page["notes_excluded"] is True
        amount = {"accounts": "balance_minor", "transactions": "amount_minor",
                  "budgets": "monthly_limit_minor", "income_streams": "expected_monthly_minor",
                  "holdings": "quantity"}[target["args"]["section"]]
        assert all((amount in record) == exact for record in page["records"])
    if not identity.startswith("health/"):
        assert "hidden-perf-body" not in json.dumps(page)


async def run(args):
    binary = args.binary.resolve(strict=True)
    target = json.loads(args.target)
    result = {
        "mcp_binary": str(binary),
        "mcp_binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "transport_harness_sha256": hashlib.sha256(TRANSPORT.read_bytes()).hexdigest(),
        "target": target,
        "samples_per_case": args.samples, "warmups_per_case": args.warmups,
        "scope": "Parent request write/drain through complete stdout frame; real MCP child, "
                 "private Unix socket and foreground native handler. Construction, parent JSON "
                 "validation, fixture loading, cold startup, physical input/paint and providers excluded.",
        "started_unix_ns": time.time_ns(),
        "cases": {},
    }
    client = await Client.start(binary, args.connection)
    try:
        initialized, _, _ = await client.request(frame("initialize"))
        assert initialized["result"]["serverInfo"]["name"] == "bruhs-ai"
        for label, query, offset, first, count, step, next_offset in cases(target["records"]):
            request = tool(target["tool"], {**target["args"], "query": query, "offset": offset})
            timings, fingerprints, sizes = [], set(), set()
            for iteration in range(args.warmups + args.samples):
                reply, elapsed, size = await client.request(request)
                check(reply, target, first, count, step, next_offset)
                if iteration >= args.warmups:
                    timings.append(elapsed)
                    sizes.add(size)
                    fingerprints.add(hashlib.sha256(json.dumps(
                        reply, sort_keys=True, separators=(",", ":")
                    ).encode()).hexdigest())
            assert len(fingerprints) == 1
            result["cases"][label] = {
                "samples_ms": timings, "summary_ms": summary(timings),
                "response_sha256": sorted(fingerprints), "response_bytes": sorted(sizes),
            }
    finally:
        await client.close()
    result["finished_unix_ns"] = time.time_ns()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"target": target["identity"], "cases": {
        name: case["summary_ms"] for name, case in result["cases"].items()
    }}, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--connection", type=Path, required=True,
                        help="Only the isolated native GPUI fixture supplies this connection")
    parser.add_argument("--target", required=True, help="JSON descriptor from the native fixture")
    parser.add_argument("--warmups", type=int, default=5)
    parser.add_argument("--samples", type=int, default=30)
    options = parser.parse_args()
    assert options.samples > 0 and options.warmups >= 0
    asyncio.run(run(options))
