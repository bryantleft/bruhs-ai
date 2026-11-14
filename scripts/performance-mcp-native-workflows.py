#!/usr/bin/env python3
"""Native fixture-only workflow calls; never approves, executes, or undoes records."""
import argparse
import asyncio
import hashlib
import json
from pathlib import Path
import runpy
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parent.parent
TRANSPORT = ROOT / "scripts/performance-mcp-stdio.py"
helpers = runpy.run_path(str(TRANSPORT))
Client, frame, tool, summary = (helpers[name] for name in ("Client", "frame", "tool", "summary"))


def check(reply, target):
    assert set(reply) == {"jsonrpc", "id", "result"}
    result = reply["result"]
    expected = target["expected"]
    error = "error" in expected
    assert result.get("isError", False) is error
    assert set(result) == ({"content", "isError"} if error else {"content"})
    assert len(result["content"]) == 1
    block = result["content"][0]
    assert set(block) == {"type", "text"} and block["type"] == "text"
    if error:
        assert block["text"] == expected["error"]
    else:
        data = json.loads(block["text"])
        if target["fresh"]:
            identity = data["workflow_id"]
            assert uuid.UUID(identity).version == 4
            assert block["text"].count(identity) == 1
            data["workflow_id"] = "generated-workflow-id"
            block["text"] = block["text"].replace(identity, "00000000-0000-4000-8000-000000000000")
        assert data == expected, (target["case"], data, expected)
        assert "private-capture-sentinel" not in json.dumps(data) or target["case"].startswith("Preview")
    return hashlib.sha256(json.dumps(reply, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()


async def run(args):
    target = json.loads(args.target)
    request = tool(target["tool"], target["arguments"])
    client = await Client.start(args.binary.resolve(strict=True), args.connection)
    samples, fingerprints, sizes = [], set(), set()
    started = time.time_ns()
    try:
        initialized, _, _ = await client.request(frame("initialize"))
        assert initialized["result"]["serverInfo"]["name"] == "bruhs-ai"
        for iteration in range(args.warmups + args.samples):
            print("PREPARE", flush=True)
            ready = await asyncio.to_thread(sys.stdin.readline)
            assert ready == "READY\n", "Native test fixture must prepare and acknowledge the case"
            reply, elapsed, size = await client.request(request)
            fingerprint = check(reply, target)
            if iteration >= args.warmups:
                samples.append(elapsed)
                fingerprints.add(fingerprint)
                sizes.add(size)
    finally:
        await client.close()
    assert len(fingerprints) == len(sizes) == 1
    output = {
        "mcp_binary_sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(),
        "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "transport_harness_sha256": hashlib.sha256(TRANSPORT.read_bytes()).hexdigest(),
        "target": target, "samples_ms": samples, "summary_ms": summary(samples),
        "response_sha256": sorted(fingerprints), "response_bytes": sorted(sizes),
        "warmups": args.warmups, "started_unix_ns": started, "finished_unix_ns": time.time_ns(),
        "scope": "Actual native Workspace workflow handlers over fixed production MCP stdio/private socket. Parent write/drain through complete stdout frame; preparation, fixture persistence waits, request encoding, parent JSON validation and startup excluded. One MCP process per case, serial warm requests. Fixture resets occur outside clocks. New proposal response is provisional and never approves/executes; rollback requests only create review state. Full seed save acknowledgements are outside latency. Only the generated workflow_id is normalized after UUID validation; other content and frame bytes are retained. No physical input/paint, providers, concurrency or cold-state acceptance.",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, indent=2, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--connection", type=Path, required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--warmups", type=int, default=5)
    parser.add_argument("--samples", type=int, default=30)
    asyncio.run(run(parser.parse_args()))
