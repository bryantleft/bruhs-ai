"""Order timestamps.

Shops send `created_at` as an ISO 8601 string with an explicit UTC offset,
in whatever offset the shop's till uses:

    "2026-03-01T10:15:30+01:00"    (Berlin)
    "2026-03-01T09:15:30Z"         (London; the same instant as the above)
    "2026-03-01T20:15:30.25+11:00" (Sydney, with a fraction of a second)

The offset is part of the timestamp: two strings that denote the same
moment are the same instant, whatever offsets they are written in.
"""

import calendar
from datetime import datetime


def parse_ts(text):
    """Parse a `created_at` string into an aware `datetime`.

    Raises `ValueError` for a string without a UTC offset.
    """
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    dt = datetime.fromisoformat(text)
    if dt.tzinfo is None:
        raise ValueError(f"timestamp without a UTC offset: {text!r}")
    return dt


def to_micros(text):
    """The instant `text` denotes, as whole microseconds since
    1970-01-01T00:00:00Z. Exact (no floating point), and equal for any two
    strings that denote the same instant."""
    dt = parse_ts(text)
    # utctimetuple: timetuple() is the wall time in the string's own offset.
    return calendar.timegm(dt.utctimetuple()) * 1_000_000 + dt.microsecond
