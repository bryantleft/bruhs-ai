"""Grade a chat task's captured reply.

usage: answer.py REPLY_FILE KIND EXPECTED...

The reply's answer is the text after the last line that starts with
`ANSWER:` (case-insensitive; markdown emphasis, backticks and a leading list
marker or `>` are ignored). Exits 0 when it matches, 1 when it does not, and
prints what it found either way.

KIND:

  text EXPECTED...         every EXPECTED value appears in the answer as a
                           whole word (case-insensitive), and the numbers and
                           dates in the answer are exactly the numeric
                           EXPECTED values, in order ("Eli, floor 4" and
                           "2027-06-07 (Monday)" pass; "Eli, 4 or 2" fails)
  numbers TOL EXPECTED...  the answer split on commas has one part per
                           EXPECTED value; the first number in each part is
                           within TOL of it; units may follow ("5.40 L/100 km")
"""

import re
import sys

LINE = re.compile(r"^[\s>*_`#-]*answer\s*[:：]\s*(.*)$", re.IGNORECASE)
NUMBER = re.compile(r"[-+]?\d+(?:\.\d+)?")
TOKEN = re.compile(r"\d[\d.-]*\d|\d|[^\W\d_]+")


def is_numeric(token):
    return token[:1].isdigit()


def text_matches(answer, expected):
    tokens = [t.lower() for t in TOKEN.findall(answer)]
    want = [e.lower() for e in expected]
    numbers = [t for t in tokens if is_numeric(t)]
    return all(w in tokens for w in want) and numbers == [w for w in want if is_numeric(w)]


def final_answer(text):
    found = None
    for line in text.splitlines():
        match = LINE.match(line)
        if match:
            found = match.group(1)
    if found is None:
        return None
    return found.replace("**", "").replace("__", "").replace("`", "").strip().rstrip(".").strip()


def main():
    path, kind, expected = sys.argv[1], sys.argv[2], sys.argv[3:]
    try:
        with open(path, encoding="utf-8", errors="replace") as handle:
            text = handle.read()
    except FileNotFoundError:
        print(f"no reply file {path}")
        return 1
    answer = final_answer(text)
    if answer is None:
        print("no 'ANSWER:' line in the reply")
        return 1
    print(f"answer: {answer!r}")
    if kind == "text":
        ok = text_matches(answer, expected)
    elif kind == "numbers":
        parts = answer.split(",")
        tolerance, want = float(expected[0]), [float(e) for e in expected[1:]]
        got = [NUMBER.search(part) for part in parts]
        ok = (
            len(got) == len(want)
            and all(got)
            and all(abs(float(g.group()) - w) <= tolerance for g, w in zip(got, want))
        )
    else:
        print(f"unknown kind {kind}")
        return 2
    print("expected: " + " ".join(expected) + ("  -> match" if ok else "  -> MISMATCH"))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
