"""Token and turn metrics from one harness run's log.

usage: metrics.py FORMAT LOG
       metrics.py --reply FORMAT LOG

FORMAT is claude | codex | bruhs-claude | bruhs-codex | aider | opencode,
bruhs-review-claude | bruhs-review-codex for a review-mode run (LOG is turn
1's log; the review and turn 2 are read from the sibling files
LOG-stem.review.json, .review.time, .review.body.json and .t2.log), or
t3-claude | t3-codex for T3 Code (LOG is `t3 thread start --json` output;
tokens come from the provider's session logs in the workdir named by the
sibling LOG-stem.cwd).

--reply prints the run's final reply text (Claude `result.result`, Codex's
last `agent_message`, bruhs's `result` or last text): chat tasks are graded
from it and review mode sends it to the reviewer. Otherwise prints one
tab-separated line:

    tokens  peak_context  turns  tool_calls  note

`-` marks a value the harness does not expose. Definitions (README.md has
the long form):

- tokens: everything the provider billed for the run — input, cache reads,
  cache writes and output, summed over every model request.
- peak_context: the largest prompt (input incl. cached) of any one request.
- turns: model requests (one per assistant message / codex token_count).
- tool_calls: tool invocations the agent made.
"""

import glob
import json
import os
import re
import sys


def read_jsonl(path):
    rows = []
    with open(path, encoding="utf-8", errors="replace") as handle:
        for line in handle:
            line = line.strip()
            if line.startswith("data:"):
                line = line[5:].strip()
            if not line.startswith("{"):
                continue
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError:
                continue
    return rows


def n(value):
    return int(value or 0)


def claude_prompt(usage):
    return (
        n(usage.get("input_tokens"))
        + n(usage.get("cache_read_input_tokens"))
        + n(usage.get("cache_creation_input_tokens"))
    )


def claude_total(usage):
    return claude_prompt(usage) + n(usage.get("output_tokens"))


def claude_transcript_messages(rows):
    """Assistant messages by id (the last copy of each: the CLI emits one
    line per content block, all with the same id and usage)."""
    messages = {}
    for row in rows:
        if row.get("type") != "assistant":
            continue
        message = row.get("message") or {}
        key = message.get("id") or id(row)
        messages[key] = message
    return messages


def count_tool_uses(messages):
    ids = set()
    for message in messages.values():
        for block in message.get("content") or []:
            if isinstance(block, dict) and block.get("type") == "tool_use":
                ids.add(block.get("id"))
    return len(ids)


def claude_metrics(rows):
    messages = claude_transcript_messages(rows)
    peaks = [claude_prompt(m.get("usage") or {}) for m in messages.values()]
    results = [row for row in rows if row.get("type") == "result"]
    note = ""
    if results:
        result = results[-1]
        tokens = claude_total(result.get("usage") or {})
        # Distinct assistant messages = model requests. `num_turns` counts
        # differently (it ran 2-4 higher) and bruhs has no equivalent.
        turns = len(messages)
        if result.get("is_error"):
            note = f"result error: {result.get('subtype', '')}".strip()
    else:
        tokens = sum(claude_total(m.get("usage") or {}) for m in messages.values())
        turns = len(messages)
        note = "no result event (killed?); tokens summed from assistant messages"
    return tokens, max(peaks) if peaks else "-", turns, count_tool_uses(messages), note


def codex_rollout(thread_id):
    """The codex session rollout for `thread_id`: its per-request
    token_count events give the peak context and request count."""
    if not thread_id:
        return None
    home = os.environ.get("CODEX_HOME") or os.path.expanduser("~/.codex")
    hits = glob.glob(os.path.join(home, "sessions", "*", "*", "*", f"rollout-*{thread_id}.jsonl"))
    return hits[0] if hits else None


def codex_requests(thread_id):
    """(peak, turns, spent) from the rollout; spent is the last running
    total's input + output, for runs whose stream lost its final usage."""
    path = codex_rollout(thread_id)
    if not path:
        return None, None, None
    peak, turns, last_total, spent = 0, 0, None, None
    for row in read_jsonl(path):
        payload = row.get("payload") or {}
        if payload.get("type") != "token_count" or not payload.get("info"):
            continue
        info = payload["info"]
        total = (info.get("total_token_usage") or {}).get("total_tokens")
        if total == last_total:
            continue  # the same request reported again
        last_total = total
        turns += 1
        peak = max(peak, n((info.get("last_token_usage") or {}).get("input_tokens")))
        spent = codex_usage_spent(info.get("total_token_usage") or {})
    return (peak or None), (turns or None), spent


def codex_usage_spent(usage):
    # input_tokens already includes cached_input_tokens: do not add it again.
    return n(usage.get("input_tokens")) + n(usage.get("output_tokens"))


def bruhs_codex_spent(event):
    """A bruhs `usage` event for a Codex turn. Since the honest-meter change
    (marked by the `context_tokens` key) bruhs reports Claude-style counts:
    input excludes cache reads, and the numbers are this turn's, not the
    thread's running total. Older servers sent Codex's own shape."""
    if "context_tokens" in event:
        return (
            n(event.get("input_tokens"))
            + n(event.get("cache_read_input_tokens"))
            + n(event.get("output_tokens"))
        )
    return codex_usage_spent(event)


CODEX_TOOL_ITEMS = {"command_execution", "file_change", "mcp_tool_call", "web_search"}


def codex_metrics(rows):
    thread_id, usage, tools, note = None, None, set(), ""
    for row in rows:
        kind = row.get("type")
        if kind == "thread.started":
            thread_id = row.get("thread_id")
        elif kind == "turn.completed":
            usage = row.get("usage") or {}  # running total for the thread
        elif kind == "turn.failed":
            note = "turn failed: " + json.dumps(row.get("error"))[:120]
        elif kind == "item.completed":
            item = row.get("item") or {}
            if item.get("type") in CODEX_TOOL_ITEMS:
                tools.add(item.get("id"))
    peak, turns, rollout_spent = codex_requests(thread_id)
    tokens = codex_usage_spent(usage) if usage is not None else "-"
    if usage is None:
        note = note or "no turn.completed event (killed?)"
        if rollout_spent is not None:
            tokens = rollout_spent
            note += "; tokens from the session rollout"
    return tokens, peak or "-", turns or "-", len(tools), note


def bruhs_events(rows):
    return [row for row in rows if row.get("envelope") == "query"]


def claude_session_transcript(session_id):
    """Claude Code's own transcript of `session_id`, which carries every
    assistant message with its usage."""
    if not session_id:
        return None
    root = os.environ.get("CLAUDE_CONFIG_DIR") or os.path.expanduser("~/.claude")
    hits = glob.glob(os.path.join(root, "projects", "*", f"{session_id}.jsonl"))
    return hits[0] if hits else None


def bruhs_session_id(rows):
    for event in bruhs_events(rows):
        if event.get("type") in ("init", "result") and event.get("session_id"):
            return event["session_id"]
    return None


def bruhs_claude_metrics(rows, use_session=True):
    events = bruhs_events(rows)
    session = None
    tokens, note = None, ""
    tools = {e.get("id") for e in events if e.get("type") == "tool_call"}
    stream_peaks = [claude_prompt(e) for e in events if e.get("type") == "usage"]
    for event in events:
        if event.get("type") in ("init", "result") and event.get("session_id"):
            session = event["session_id"]
        if event.get("type") == "result":
            tokens = claude_total(event.get("usage") or {})
            if event.get("is_error"):
                note = "result error"
        if event.get("type") == "error":
            note = "error: " + str(event.get("message"))[:120]
    peak, turns = (max(stream_peaks) if stream_peaks else None), len(stream_peaks) or None
    transcript = claude_session_transcript(session) if use_session else None
    if transcript:
        messages = claude_transcript_messages(read_jsonl(transcript))
        if messages:
            peak = max(claude_prompt(m.get("usage") or {}) for m in messages.values())
            turns = len(messages)
    if tokens is None:
        note = note or "no result event"
    return (tokens if tokens is not None else "-"), peak or "-", turns or "-", len(tools), note


def bruhs_codex_metrics(rows, use_session=True):
    events = bruhs_events(rows)
    session, usage, note = None, None, ""
    tools = {e.get("id") for e in events if e.get("type") == "tool_call"}
    for event in events:
        if event.get("type") == "init":
            session = event.get("session_id")
        elif event.get("type") == "usage":
            usage = event
        elif event.get("type") == "error":
            note = "error: " + str(event.get("message"))[:120]
    peak, turns, rollout_spent = codex_requests(session) if use_session else (None, None, None)
    tokens = bruhs_codex_spent(usage) if usage else "-"
    if usage is None:
        note = note or "stream ended without a usage event"
        if rollout_spent is not None:
            tokens = rollout_spent
            note += "; tokens from the session rollout (a lower bound if the turn was cut short)"
    return tokens, peak or "-", turns or "-", len(tools), note


# ---------------------------------------------------------------- replies


def claude_reply(rows):
    results = [row for row in rows if row.get("type") == "result"]
    if results and isinstance(results[-1].get("result"), str):
        return results[-1]["result"]
    messages = list(claude_transcript_messages(rows).values())
    for message in reversed(messages):
        text = "".join(
            block.get("text", "")
            for block in message.get("content") or []
            if isinstance(block, dict) and block.get("type") == "text"
        )
        if text:
            return text
    return ""


def codex_reply(rows):
    text = ""
    for row in rows:
        item = row.get("item") or {}
        if row.get("type") == "item.completed" and item.get("type") == "agent_message":
            text = item.get("text") or ""
    return text


def bruhs_reply(rows, provider):
    """The turn's final reply. Claude turns end with a `result` carrying it;
    Codex turns have none, and each Codex `text` event is a whole message,
    so the last one is the reply. Claude `text` events are deltas: without a
    result, join the ones after the last tool event."""
    events = bruhs_events(rows)
    for event in reversed(events):
        if event.get("type") == "result" and isinstance(event.get("result"), str) and event["result"]:
            return event["result"]
    texts = []
    for event in events:
        if event.get("type") in ("tool_call", "tool_result"):
            texts = []
        elif event.get("type") == "text":
            texts.append(event.get("text") or "")
    if provider == "codex":
        return texts[-1] if texts else ""
    return "".join(texts)


def reply_text(fmt, path):
    if fmt.startswith("bruhs-review-"):
        second = sibling(path, ".t2.log")
        if os.path.exists(second):
            path = second
        fmt = "bruhs-" + fmt[len("bruhs-review-"):]
    if not os.path.exists(path):
        return ""
    rows = read_jsonl(path)
    if fmt.startswith("t3-"):
        return str((rows[-1] if rows else {}).get("reply") or "")
    if fmt == "claude":
        return claude_reply(rows)
    if fmt == "codex":
        return codex_reply(rows)
    if fmt == "bruhs-claude":
        return bruhs_reply(rows, "claude")
    if fmt == "bruhs-codex":
        return bruhs_reply(rows, "codex")
    return ""


# ---------------------------------------------------------------- review mode


def sibling(path, suffix):
    """`<stem>.log` -> `<stem><suffix>`."""
    stem = path[:-4] if path.endswith(".log") else path
    return stem + suffix


def read_json(path):
    try:
        with open(path, encoding="utf-8") as handle:
            return json.load(handle)
    except (OSError, ValueError):
        return None


def read_text(path):
    try:
        with open(path, encoding="utf-8") as handle:
            return handle.read().strip()
    except OSError:
        return ""


def claude_project_dir(cwd):
    """Claude Code's transcript directory for a working directory: the path
    with every character outside [A-Za-z0-9] replaced by '-'."""
    root = os.environ.get("CLAUDE_CONFIG_DIR") or os.path.expanduser("~/.claude")
    return os.path.join(root, "projects", re.sub(r"[^A-Za-z0-9]", "-", cwd))


def codex_rollouts_in(cwd):
    """Rollouts whose session_meta says they ran in `cwd`."""
    home = os.environ.get("CODEX_HOME") or os.path.expanduser("~/.codex")
    hits = []
    for path in glob.glob(os.path.join(home, "sessions", "*", "*", "*", "rollout-*.jsonl")):
        try:
            with open(path, encoding="utf-8", errors="replace") as handle:
                first = json.loads(handle.readline() or "{}")
        except (OSError, ValueError):
            continue
        payload = first.get("payload") or {}
        if first.get("type") == "session_meta" and payload.get("cwd") == cwd:
            hits.append((path, payload.get("id")))
    return hits


def reviewer_tokens(reviewer, cwd, exclude):
    """(tokens, how) for the reviewer's own run, read from its session log
    in `cwd`, ignoring the thread's own sessions `exclude`. None when the
    reviewer left no log."""
    if not cwd:
        return None, "no review request body"
    if reviewer == "claude":
        found = [
            path
            for path in glob.glob(os.path.join(claude_project_dir(cwd), "*.jsonl"))
            if os.path.basename(path)[:-6] not in exclude
        ]
        if not found:
            return None, "Claude reviewer left no transcript"
        total = 0
        for path in found:
            messages = claude_transcript_messages(read_jsonl(path))
            total += sum(claude_total(m.get("usage") or {}) for m in messages.values())
        return total, "from the reviewer's transcript (output undercounted)"
    found = [path for path, session in codex_rollouts_in(cwd) if session not in exclude]
    if not found:
        return None, "Codex reviewer is ephemeral and left no rollout"
    total = 0
    for path in found:
        spent = None
        for row in read_jsonl(path):
            payload = row.get("payload") or {}
            if payload.get("type") == "token_count" and payload.get("info"):
                spent = codex_usage_spent(payload["info"].get("total_token_usage") or {})
        total += spent or 0
    return total, "from the reviewer's rollout"


def review_metrics(fmt, path):
    """Turn 1 + review + turn 2 of a review-mode run. Both turns resume one
    provider session, whose log (transcript or rollout) then covers both, so
    peak and request count come from it once rather than per turn."""
    provider = fmt[len("bruhs-review-"):]
    base = bruhs_claude_metrics if provider == "claude" else bruhs_codex_metrics
    first = read_jsonl(path)
    second_path = sibling(path, ".t2.log")
    second = read_jsonl(second_path) if os.path.exists(second_path) else []
    runs = [rows for rows in (first, second) if rows]
    parts = [base(rows, use_session=False) for rows in runs]
    known = [p[0] for p in parts if isinstance(p[0], int)]
    tokens = sum(known) if known else "-"
    tools = sum(p[3] for p in parts if isinstance(p[3], int))
    peaks = [p[1] for p in parts if isinstance(p[1], int)]
    turns = [p[2] for p in parts if isinstance(p[2], int)]
    peak, count = (max(peaks) if peaks else "-"), (sum(turns) if turns else "-")
    sessions = []
    for rows in runs:
        session = bruhs_session_id(rows)
        if session and session not in sessions:
            sessions.append(session)
    session_peaks, session_turns = [], []
    for session in sessions:
        if provider == "claude":
            transcript = claude_session_transcript(session)
            messages = claude_transcript_messages(read_jsonl(transcript)) if transcript else {}
            if messages:
                session_peaks.append(max(claude_prompt(m.get("usage") or {}) for m in messages.values()))
                session_turns.append(len(messages))
        else:
            rollout_peak, rollout_turns, _ = codex_requests(session)
            if rollout_peak:
                session_peaks.append(rollout_peak)
                session_turns.append(rollout_turns)
    if session_peaks and len(session_peaks) == len(sessions):
        peak, count = max(session_peaks), sum(session_turns)

    notes = [p[4] for p in parts if p[4]]
    first_pass = read_text(sibling(path, ".t1.pass"))
    if first_pass in ("0", "1"):
        notes.append("turn 1 alone: " + ("PASS" if first_pass == "1" else "FAIL"))
    if len(parts) == 2 and all(isinstance(p[0], int) for p in parts):
        notes.append(f"turn tokens {parts[0][0]:,} + {parts[1][0]:,}")
    review = read_json(sibling(path, ".review.json"))
    review_time = read_text(sibling(path, ".review.time")) or "?"
    if review is None:
        notes.append("no review (" + (read_text(sibling(path, ".review.skip")) or "not run") + ")")
    elif review.get("error"):
        notes.append(f"review error after {review_time}s: {str(review['error'])[:100]}")
    else:
        reviewer = review.get("reviewer") or "?"
        body = read_json(sibling(path, ".review.body.json")) or {}
        usage = review.get("usage")
        if usage:
            # Claude-style counts from the server (input excludes cache reads).
            extra = sum(n(usage.get(key)) for key in (
                "input_tokens", "output_tokens",
                "cache_read_input_tokens", "cache_creation_input_tokens"))
            how = "(reported by the server)"
        else:
            extra, how = reviewer_tokens(reviewer, body.get("project_path"), set(sessions))
        if extra is not None and isinstance(tokens, int):
            tokens += extra
            notes.append(f"review by {reviewer} {review_time}s, {extra:,} tokens {how}")
        else:
            notes.append(f"review by {reviewer} {review_time}s; tokens = turns only ({how})")
        if not second:
            notes.append("no turn 2")
    return tokens, peak, count, tools, "; ".join(notes)


def scaled(number, suffix):
    value = float(number)
    return int(value * {"k": 1e3, "m": 1e6}.get(suffix.lower(), 1))


def aider_metrics(path):
    with open(path, encoding="utf-8", errors="replace") as handle:
        text = handle.read()
    sent = received = turns = peak = 0
    for match in re.finditer(r"Tokens: ([\d.]+)([kKmM]?) sent, ([\d.]+)([kKmM]?) received", text):
        prompt = scaled(match.group(1), match.group(2))
        sent += prompt
        received += scaled(match.group(3), match.group(4))
        peak = max(peak, prompt)
        turns += 1
    if not turns:
        return "-", "-", "-", "-", "no token lines in aider output"
    return sent + received, peak, turns, "-", "aider rounds token counts"


def opencode_metrics(rows):
    tokens = peak = turns = tools = 0
    for row in rows:
        part = row.get("part") or {}
        kind = row.get("type") or part.get("type")
        if kind in ("tool_use", "tool"):
            tools += 1
        usage = part.get("tokens") if kind in ("step_finish", "step-finish") else None
        if not usage:
            continue
        cache = usage.get("cache") or {}
        prompt = n(usage.get("input")) + n(cache.get("read")) + n(cache.get("write"))
        tokens += prompt + n(usage.get("output")) + n(usage.get("reasoning"))
        peak = max(peak, prompt)
        turns += 1
    if not turns:
        return "-", "-", "-", tools or "-", "no step_finish token events found"
    return tokens, peak, turns, tools, ""


# ---------------------------------------------------------------- T3 Code


CODEX_TOOL_PAYLOADS = {"function_call", "custom_tool_call", "local_shell_call"}


def t3_metrics(provider, path):
    """A T3 thread's run, read from the session logs its provider left in
    the workdir: every top-level Claude transcript there (the same scope as
    bruhs's session transcript: subagent logs are not counted for either),
    or every Codex rollout whose cwd is the workdir."""
    rows = read_jsonl(path)
    reply = rows[-1] if rows else {}
    note = ""
    if reply.get("status") != "completed":
        note = f"t3 status {reply.get('status')}: {str(reply.get('error'))[:120]}"
    cwd = read_text(sibling(path, ".cwd"))
    if not cwd:
        return "-", "-", "-", "-", note or "no .cwd file"
    if provider == "claude":
        messages = {}
        for transcript in sorted(glob.glob(os.path.join(claude_project_dir(cwd), "*.jsonl"))):
            messages.update(claude_transcript_messages(read_jsonl(transcript)))
        if not messages:
            return "-", "-", "-", "-", note or "no Claude transcript in the workdir"
        tokens = sum(claude_total(m.get("usage") or {}) for m in messages.values())
        peak = max(claude_prompt(m.get("usage") or {}) for m in messages.values())
        return tokens, peak, len(messages), count_tool_uses(messages), note
    tokens = peak = turns = tools = 0
    rollouts = codex_rollouts_in(cwd)
    for rollout, session in rollouts:
        request_peak, request_turns, spent = codex_requests(session)
        tokens += spent or 0
        peak = max(peak, request_peak or 0)
        turns += request_turns or 0
        for row in read_jsonl(rollout):
            payload = row.get("payload") or {}
            if row.get("type") == "response_item" and payload.get("type") in CODEX_TOOL_PAYLOADS:
                tools += 1
    if not rollouts:
        return "-", "-", "-", "-", note or "no Codex rollout in the workdir"
    return tokens, peak or "-", turns or "-", tools, note


def main():
    if len(sys.argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2
    if sys.argv[1] == "--reply":
        if len(sys.argv) < 4:
            print(__doc__, file=sys.stderr)
            return 2
        sys.stdout.write(reply_text(sys.argv[2], sys.argv[3]))
        return 0
    fmt, path = sys.argv[1], sys.argv[2]
    if not os.path.exists(path):
        fields = ("-", "-", "-", "-", "no log")
    elif fmt == "aider":
        fields = aider_metrics(path)
    elif fmt in ("bruhs-review-claude", "bruhs-review-codex"):
        fields = review_metrics(fmt, path)
    elif fmt in ("t3-claude", "t3-codex"):
        fields = t3_metrics(fmt[len("t3-"):], path)
    else:
        rows = read_jsonl(path)
        handler = {
            "claude": claude_metrics,
            "codex": codex_metrics,
            "bruhs-claude": bruhs_claude_metrics,
            "bruhs-codex": bruhs_codex_metrics,
            "opencode": opencode_metrics,
        }.get(fmt)
        if handler is None:
            print(f"unknown format {fmt}", file=sys.stderr)
            return 2
        fields = handler(rows)
    print("\t".join(str(field).replace("\t", " ").replace("\n", " ") for field in fields))
    return 0


if __name__ == "__main__":
    sys.exit(main())
