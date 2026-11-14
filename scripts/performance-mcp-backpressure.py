#!/usr/bin/env python3
"""Check complete MCP frames under a slow, pipelined synthetic client.

This is a functional backpressure check, not a latency benchmark. All workflows
are blocked dry runs in an isolated standalone process; no providers run.
"""
import argparse
import asyncio
import hashlib
import json
import os
from pathlib import Path
import runpy
import time

ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS = ROOT / "scripts/performance-mcp-workflows.py"
helpers = runpy.run_path(str(WORKFLOWS))
Client, frame, tool, content, preview, audit, KnownWorkflows = (
    helpers[k] for k in ["Client", "frame", "tool", "content", "preview", "audit", "KnownWorkflows"]
)


async def run(args):
    binary = args.binary.resolve(strict=True)
    env = dict(os.environ)
    env.pop("BRUHS_WORKSPACE_BRIDGE", None)
    process = await asyncio.create_subprocess_exec(
        str(binary), stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE, env=env, limit=8192,
    )
    client = Client(process)
    known = KnownWorkflows()
    result = {
        "binary": str(binary), "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "workflow_harness_sha256": hashlib.sha256(WORKFLOWS.read_bytes()).hexdigest(),
        "history": args.history, "reader_buffer_limit": 8192,
        "read_chunk_bytes": args.chunk, "read_delay_ms": args.delay_ms,
        "scope": "Functional complete-frame and current-state checks with pipelined requests and a slow reader. The small asyncio reader buffer applies real stdout backpressure. Intentional read delays, seeding and parent validation are not action latency measurements. All writes are blocked dry runs, no providers or domain changes.",
        "started_unix_ns": time.time_ns(), "frames": [],
    }
    try:
        await client.request(frame("initialize"))
        for index in range(args.history):
            operation = {"type": "create_task", "title": f"Synthetic seed {index}"}
            reply, _, _ = await client.request(tool("bruhs_write_tasks", {"operation": operation, "dry_run": True}))
            preview(reply, "tasks", operation, known)
        operation = helpers["OPERATIONS"]["tasks"]
        requests = [
            frame("tools/call", {"name": "bruhs_agent_audit", "arguments": {}}, "audit\nbefore"),
            frame("ping", request_id="ping"),
            frame("tools/call", {"name": "bruhs_write_tasks", "arguments": {"operation": {"type": "create_task", "title": " "}}}, "invalid"),
            frame("tools/call", {"name": "bruhs_write_tasks", "arguments": {"operation": operation, "dry_run": True}}, "preview"),
            frame("tools/call", {"name": "bruhs_agent_audit", "arguments": {}}, "audit\nafter"),
        ]
        process.stdin.write(b"".join(requests))
        await process.stdin.drain()
        pending = b""

        async def slow_frame():
            nonlocal pending
            parts, reads = [], 0
            while True:
                data = pending
                if not data:
                    data = await asyncio.wait_for(process.stdout.read(args.chunk), 10)
                    assert data, "stdout ended before the complete frame"
                    reads += 1
                    if args.delay_ms:
                        await asyncio.sleep(args.delay_ms / 1000)
                fragment, delimiter, pending = data.partition(b"\n")
                parts.append(fragment)
                if delimiter:
                    return b"".join(parts) + b"\n", reads

        for index, request in enumerate(requests):
            encoded, reads = await slow_frame()
            reply, requested = json.loads(encoded), json.loads(request)
            assert reply["jsonrpc"] == "2.0" and reply["id"] == requested["id"]
            if index in [0, 4]:
                normalized = audit(reply, known)
            elif index == 1:
                assert reply["result"] == {} and set(reply) == {"jsonrpc", "id", "result"}
                normalized = json.dumps(reply, sort_keys=True)
            elif index == 2:
                normalized = content(reply)
                assert reply["result"]["isError"] is True
            else:
                normalized = preview(reply, "tasks", operation, known)
            result["frames"].append({"id": requested["id"], "response_bytes": len(encoded),
                                     "reads": reads, "normalized_response_sha256": hashlib.sha256(normalized.encode()).hexdigest()})
        assert not pending and len(known) == args.history + 1
        reply, _, _ = await client.request(frame("ping", request_id="resumed"))
        assert reply["result"] == {}
        result["final_workflows"] = len(known)
    finally:
        await client.close()
    result["finished_unix_ns"] = time.time_ns()
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print("Five ordered pipelined frames, complete audit snapshots, invalid-write rejection and resumed ping pass.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--history", type=int, default=10000)
    parser.add_argument("--chunk", type=int, default=4096)
    parser.add_argument("--delay-ms", type=float, default=1.0)
    args = parser.parse_args()
    assert args.history >= 0 and args.chunk > 0 and args.delay_ms >= 0
    asyncio.run(run(args))
