#!/usr/bin/env python3
"""Synthetic standalone preview/history timings; never applies domain writes."""
import argparse
import asyncio
import hashlib
import json
import os
from pathlib import Path
import re
import runpy
import time
import uuid

ROOT = Path(__file__).resolve().parent.parent
TRANSPORT = ROOT / "scripts/performance-mcp-stdio.py"
helpers = runpy.run_path(str(TRANSPORT))
Client, frame, tool, summary = (helpers[k] for k in ["Client", "frame", "tool", "summary"])
TEXT = 'Synthetic Café ΟΣ 👩🏽‍💻 "quoted" \\ text'
BODY = TEXT + '\n literal "created_at_ms": 7\tkept in the body'
OPERATIONS = {
    "chat": {"type": "create_chat_thread", "title": TEXT},
    "tasks": {"type": "create_task", "title": TEXT},
    "calendar": {"type": "create_calendar_event", "title": TEXT, "start_at_ms": 0.0, "end_at_ms": 60000.0},
    "meetings": {"type": "create_meeting_note", "title": TEXT, "body": BODY},
    "knowledge": {"type": "create_knowledge_note", "title": TEXT, "body": BODY},
    "inbox": {"type": "draft_inbox_message", "recipient": "synthetic@example.invalid", "body": BODY},
    "people": {"type": "create_person_reminder", "person_id": "synthetic-person", "person": TEXT, "reminder": BODY},
    "finance": {"type": "add_finance_transaction", "merchant": TEXT, "category": "Synthetic", "amount_minor": "-125"},
    "health": {"type": "log_workout", "activity": TEXT, "minutes": 30},
    "capture": {"type": "add_capture", "content": BODY},
}
UUID_RE = re.compile(r'[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}')
TIME_RE = re.compile(r'(?<!\\)("(?:created_at_ms|updated_at_ms|occurred_at_ms)"\s*:\s*)(\d+)')


class KnownWorkflows(list):
    def __init__(self):
        super().__init__()
        self.identities = set()


class HistoryClient(Client):
    @classmethod
    async def start(cls, binary):
        env = dict(os.environ)
        env.pop("BRUHS_WORKSPACE_BRIDGE", None)
        process = await asyncio.create_subprocess_exec(
            "/usr/bin/time", "-l", str(binary), stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
            env=env, limit=64 * 1024 * 1024,
        )
        return cls(process)

    async def finish(self):
        self.process.stdin.close()
        try:
            await asyncio.wait_for(self.process.wait(), 15)
            stderr = (await self.process.stderr.read()).decode(errors="replace")
            assert self.process.returncode == 0, stderr
            return stderr
        finally:
            if self.process.returncode is None:
                self.process.kill()
                await self.process.wait()


def content(reply):
    assert "error" not in reply, reply
    assert set(reply) == {"jsonrpc", "id", "result"}
    assert set(reply["result"]) in [{"content"}, {"content", "isError"}]
    assert len(reply["result"]["content"]) == 1
    block = reply["result"]["content"][0]
    assert set(block) == {"type", "text"} and block["type"] == "text"
    return block["text"]


def preview(reply, domain, operation, known):
    text = content(reply)
    assert "isError" not in reply["result"]
    data = json.loads(text)
    assert set(data) == {"dry_run", "workflow_id", "allowed", "changes", "blocked_domains"}
    assert data["dry_run"] is True and data["allowed"] is False
    assert data["blocked_domains"] == [domain]
    assert len(data["changes"]) == 1 and isinstance(data["changes"][0], str)
    identity = data["workflow_id"]
    assert uuid.UUID(identity).version == 4 and identity not in known.identities
    known.append((identity, domain, operation))
    known.identities.add(identity)
    assert text.count(identity) == 1
    return text.replace(identity, "10000000-0000-4000-8000-000000000000")


def audit(reply, known):
    text = content(reply)
    assert "isError" not in reply["result"]
    state = json.loads(text)
    assert set(state) == {"permissions", "workflows", "jobs", "audit"}
    assert state["jobs"] == []
    assert {(c["domain"], c["access"]) for c in state["permissions"]["grants"]} == {
        (domain, "read") for domain in OPERATIONS if domain not in ["finance", "health"]
    }
    assert len(state["workflows"]) == len(state["audit"]) == len(known)
    replacements, audit_ids = {}, set()
    for index, (workflow, event, expected) in enumerate(zip(state["workflows"], state["audit"], known)):
        identity, domain, operation = expected
        assert workflow["id"] == event["workflow_id"] == identity
        assert workflow["title"] == f"MCP {domain.title()} write"
        assert workflow["status"] == "previewed" and workflow["previewed_operations"] is None
        assert len(workflow["steps"]) == 1
        step = workflow["steps"][0]
        assert step["operation"] == operation and step["status"] == "pending"
        assert step["target_id"] is None and step["error"] is None and step["target_snapshot"] is None
        assert event["kind"] == "previewed" and event["message"] == "Dry run blocked 1 domain grant(s)."
        stamp = workflow["created_at_ms"]
        assert stamp == workflow["updated_at_ms"] == event["occurred_at_ms"] and len(str(stamp)) == 13
        assert uuid.UUID(event["id"]).version == 4 and event["id"] not in audit_ids
        audit_ids.add(event["id"])
        replacements[identity] = f"10000000-0000-4000-8000-{index:012x}"
        replacements[event["id"]] = f"20000000-0000-4000-8000-{index:012x}"
    count = 0
    def replace_id(match):
        nonlocal count
        count += 1
        return replacements[match[0]]
    normalized = UUID_RE.sub(replace_id, text)
    assert count == len(known) * 3
    normalized, count = TIME_RE.subn(lambda m: m[1] + "1700000000000", normalized)
    assert count == len(known) * 3
    return normalized


async def run(args):
    binary = args.binary.resolve(strict=True)
    result = {"binary": str(binary), "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              "transport_harness_sha256": hashlib.sha256(TRANSPORT.read_bytes()).hexdigest(),
              "scope": "Real standalone MCP process, request write/drain through complete stdout frame. Parent JSON validation and identity/time normalization excluded. UUIDs and timestamps are validated and normalized in their known fields; all other text formatting, data, array order and frame sizes are retained. Inputs are synthetic, every write is dry_run=true and blocked by default Write grants. No providers, domain writes, messages or jobs are started.",
              "samples_per_case": args.samples, "warmups_per_case": args.warmups,
              "started_unix_ns": time.time_ns(), "histories": {}}
    for history in args.histories:
        client = await HistoryClient.start(binary)
        known = KnownWorkflows()
        cases = {}
        try:
            reply, _, _ = await client.request(frame("initialize"))
            assert reply["result"]["serverInfo"]["name"] == "bruhs-ai"
            started = time.perf_counter_ns()
            for index in range(history):
                operation = {"type": "create_task", "title": f"Synthetic seed {index}"}
                reply, _, _ = await client.request(tool("bruhs_write_tasks", {"operation": operation, "dry_run": True}))
                preview(reply, "tasks", operation, known)
            seed_ms = (time.perf_counter_ns() - started) / 1_000_000
            controls = [("audit", tool("bruhs_agent_audit", {}), "audit")]
            controls += [("preview-" + domain, tool("bruhs_write_" + domain, {"operation": operation, "dry_run": True}), domain) for domain, operation in OPERATIONS.items()]
            controls += [("rejected-apply", tool("bruhs_write_tasks", {"operation": OPERATIONS["tasks"], "dry_run": False}), "error"),
                         ("invalid-operation", tool("bruhs_write_tasks", {"operation": {"type": "create_task", "title": " "}}), "error"),
                         ("read-chat", tool("bruhs_read_chat", {}), "empty"),
                         ("list-threads", tool("bruhs_list_threads", {}), "empty"),
                         ("list-tasks", tool("bruhs_list_tasks", {"thread_id": "synthetic-missing"}), "empty"),
                         ("ping", frame("ping"), "ping"), ("tools-list", frame("tools/list"), "catalog")]
            for name, request, kind in controls:
                initial = len(known)
                samples, fingerprints, sizes = [], set(), set()
                for iteration in range(args.warmups + args.samples):
                    reply, elapsed, size = await client.request(request)
                    if kind == "audit": normalized = audit(reply, known)
                    elif kind in OPERATIONS: normalized = preview(reply, kind, OPERATIONS[kind], known)
                    elif kind in ["empty", "error"]:
                        normalized = content(reply)
                        if kind == "error": assert reply["result"]["isError"] is True
                        else: assert json.loads(normalized) == [] and "isError" not in reply["result"]
                    elif kind == "ping":
                        assert reply["result"] == {}; normalized = json.dumps(reply, sort_keys=True)
                    else:
                        assert any(t["name"] == "bruhs_agent_audit" for t in reply["result"]["tools"])
                        normalized = json.dumps(reply, sort_keys=True)
                    if iteration >= args.warmups:
                        samples.append(elapsed); sizes.add(size)
                        fingerprints.add(hashlib.sha256(normalized.encode()).hexdigest())
                assert len(fingerprints) == 1, (history, name)
                cases[name] = {"history_before": initial, "history_after": len(known), "samples_ms": samples,
                               "summary_ms": summary(samples), "response_bytes": sorted(sizes), "normalized_response_sha256": sorted(fingerprints)}
            reply, _, _ = await client.request(tool("bruhs_agent_audit", {})); audit(reply, known)
            assert len(known) == history + len(OPERATIONS) * (args.warmups + args.samples)
        finally:
            resources = await client.finish()
        result["histories"][str(history)] = {"seed_ms_excluded": seed_ms, "cases": cases, "final_workflows": len(known), "resource_report_whole_process": resources}
        print(f"History {history}: {len(cases)} full-frame cases, contents/state/blocked-write checks passed", flush=True)
    result["finished_unix_ns"] = time.time_ns()
    args.output.write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--histories", type=int, nargs="+", default=[0, 128, 1024, 10000])
    parser.add_argument("--samples", type=int, default=30)
    parser.add_argument("--warmups", type=int, default=5)
    args = parser.parse_args()
    assert args.samples > 0 and args.warmups >= 0 and all(n >= 0 for n in args.histories)
    asyncio.run(run(args))
