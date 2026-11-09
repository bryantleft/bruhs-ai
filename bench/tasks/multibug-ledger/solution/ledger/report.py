"""Plain-text statements."""

import datetime

from .accounts import Ledger  # noqa: F401  (re-exported for callers)


def format_cents(cents):
    """Cents as a dollar string with two decimals and "," thousands separators:
    123456 -> "1,234.56", 5 -> "0.05", 0 -> "0.00", -1234 -> "-12.34",
    -5 -> "-0.05"."""
    sign = "-" if cents < 0 else ""
    dollars, rest = divmod(abs(cents), 100)
    return f"{sign}{dollars:,}.{rest:02d}"


def statement_lines(ledger, account, start, end):
    """The statement for `account` from `start` to `end` inclusive:

        Opening balance <balance as of the day before start>
        <day> <memo> <amount> <running balance>     (one line per entry)
        Closing balance <balance as of end>

    Fields are separated by two spaces; amounts use format_cents.
    """
    opening = ledger.balance(account, start - datetime.timedelta(days=1))
    lines = [f"Opening balance  {format_cents(opening)}"]
    running = opening
    for entry in ledger.entries_between(account, start, end):
        running += entry.cents
        lines.append(
            f"{entry.day.isoformat()}  {entry.memo}  {format_cents(entry.cents)}  {format_cents(running)}"
        )
    lines.append(f"Closing balance  {format_cents(ledger.balance(account, end))}")
    return lines
