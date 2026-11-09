"""Glue for review mode (tools bruhs-review, bruhs-codex-review) and the
bruhs request bodies.

usage:
  review.py turn-body OUT PROMPT_FILE PROJECT THREAD MODEL PERMISSION SESSION
      POST /api/agent body. Empty PROJECT sends no project_path (a chat in
      the home directory, lean context); empty SESSION starts a fresh
      session, otherwise the turn resumes SESSION.
  review.py session LOG
      the provider session id from a bruhs SSE log (init or result event)
  review.py review-body OUT PROMPT_FILE REPLY_FILE PROJECT PROVIDER
      POST /api/agent/second-opinion body; PROVIDER is the thread's
  review.py followup REVIEW_JSON OUT
      the follow-up prompt for turn 2, the same text bruhs's composer
      drafts after /second-opinion (bruhs_models::logic::handoff::
      second_opinion_followup). Exits 1, printing why, when the response
      carries an error or no review.
"""

import json
import sys

PROVIDER_NAMES = {"claude": "Claude", "codex": "Codex"}
VERDICT_HOLDS_UP = "verdict: holds up"


def holds_up(review):
    """The reviewer's first non-empty line is `VERDICT: holds up`
    (markdown `*` and backticks ignored), as bruhs checks it."""
    for line in review.splitlines():
        line = line.strip().strip("*`").strip()
        if line:
            return line.lower() == VERDICT_HOLDS_UP
    return False


def read(path):
    with open(path, encoding="utf-8", errors="replace") as handle:
        return handle.read()


def write_json(path, value):
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(value, handle)


def turn_body(out, prompt, project, thread, model, permission, session):
    body = {
        "prompt": read(prompt),
        "thread_id": thread,
        "model": model,
        "permission_mode": permission,
    }
    if project:
        body["project_path"] = project
    if session:
        body["session_id"] = session
    else:
        body["fresh_session"] = True
    write_json(out, body)
    return 0


def session(log):
    found = ""
    for line in read(log).splitlines():
        line = line.strip()
        if line.startswith("data:"):
            line = line[5:].strip()
        if not line.startswith("{"):
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("envelope") == "query" and event.get("type") in ("init", "result"):
            found = event.get("session_id") or found
    print(found)
    return 0


def review_body(out, prompt, reply, project, provider):
    body = {
        "thread_provider": provider,
        "task": read(prompt),
        "answer": read(reply),
        "focus": "",
    }
    # Empty PROJECT: a chat with no project; the server reviews from the
    # home directory (an explicit empty path is a 400).
    if project:
        body["project_path"] = project
    write_json(out, body)
    return 0


def followup(review_json, out):
    try:
        response = json.loads(read(review_json))
    except (OSError, ValueError) as error:
        print(f"unreadable review response: {error}")
        return 1
    if response.get("error"):
        print(f"review error: {response['error']}")
        return 1
    review = (response.get("review") or "").strip()
    if not review:
        print("empty review")
        return 1
    reviewer = PROVIDER_NAMES.get(response.get("reviewer"), str(response.get("reviewer")))
    if holds_up(review):
        # bruhs shows a notice and drafts no follow-up when the reviewer
        # agrees (bruhs_models::logic::handoff::review_holds_up).
        print(f"{reviewer} found no problems: no follow-up turn")
        return 1
    with open(out, "w", encoding="utf-8") as handle:
        handle.write(
            f"A second reviewer ({reviewer}) looked at your last answer:\n\n{review}\n\n"
            "Check each point (against the code, where there is code). Fix what is right, "
            "and say briefly why you disagree with the rest. If your answer changes, state "
            "the final answer again in full."
        )
    return 0


def main():
    commands = {
        "turn-body": (turn_body, 7),
        "session": (session, 1),
        "review-body": (review_body, 5),
        "followup": (followup, 2),
    }
    if len(sys.argv) < 2 or sys.argv[1] not in commands:
        print(__doc__, file=sys.stderr)
        return 2
    handler, arity = commands[sys.argv[1]]
    args = sys.argv[2:]
    if len(args) != arity:
        print(__doc__, file=sys.stderr)
        return 2
    return handler(*args)


if __name__ == "__main__":
    sys.exit(main())
