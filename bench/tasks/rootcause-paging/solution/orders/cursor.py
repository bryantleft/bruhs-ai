"""Page cursors.

A cursor is an opaque string the client sends back to get the next page.
It encodes the `sort_key` (store.py) of the last order on the page it came
from, so the next page is simply "the orders whose key is greater". That
keeps paging correct while orders are added or removed between requests,
which offsets (skip the first N) do not.
"""

import base64
import json


class InvalidCursor(ValueError):
    """The string is not a cursor that `encode_cursor` produced."""


def encode_cursor(key):
    """The cursor for a page whose last order has sort key `key`."""
    return base64.urlsafe_b64encode(json.dumps(list(key)).encode()).decode()


def decode_cursor(token):
    """The sort key a cursor encodes. Raises `InvalidCursor` for any string
    that `encode_cursor` could not have produced."""
    if not isinstance(token, str):
        raise InvalidCursor("a cursor is a string")
    try:
        payload = json.loads(base64.urlsafe_b64decode(token.encode("ascii")).decode("utf-8"))
    except (ValueError, UnicodeError) as exc:
        raise InvalidCursor(f"not a cursor: {token!r}") from exc
    if (
        not isinstance(payload, list)
        or len(payload) != 2
        or not all(isinstance(x, int) and not isinstance(x, bool) for x in payload)
    ):
        raise InvalidCursor(f"not a cursor: {token!r}")
    return tuple(payload)
