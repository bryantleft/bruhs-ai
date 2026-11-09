"""The order listing endpoint (GET /orders)."""

from .cursor import decode_cursor, encode_cursor
from .store import sort_key


def list_orders(store, limit=20, cursor=None):
    """One page of the order listing.

    Returns `{"orders": [...], "next_cursor": ...}`. `orders` holds at most
    `limit` orders, in listing order (newest first, see `store.sort_key`).
    The first page is the one without a cursor; passing a page's
    `next_cursor` returns the orders that come right after that page.
    `next_cursor` is `None` exactly when the page ends with the last order
    of the listing (so a client never fetches an empty page), otherwise a
    string.

    Following the cursors from the first page visits every order that
    existed when the first page was fetched and still exists, exactly once
    and in listing order, even when orders are added or removed between
    requests.

    `limit` must be an int from 1 to 100 (`ValueError` otherwise). A cursor
    that `encode_cursor` did not produce raises `cursor.InvalidCursor`.
    """
    if not isinstance(limit, int) or not 1 <= limit <= 100:
        raise ValueError("limit must be an int from 1 to 100")
    orders = store.listing()
    if cursor is not None:
        after = decode_cursor(cursor)
        orders = [o for o in orders if sort_key(o) > after]
    page = orders[:limit]
    next_cursor = encode_cursor(sort_key(page[-1])) if len(page) == limit else None
    return {"orders": page, "next_cursor": next_cursor}
