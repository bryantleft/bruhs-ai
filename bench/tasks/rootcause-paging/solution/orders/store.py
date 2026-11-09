"""In-memory order storage."""

from .timeutil import to_micros


def sort_key(order):
    """Where `order` appears in a listing: smaller keys come first.

    Listings show the newest order first: by the instant of `created_at`,
    latest first; orders created at the same instant are listed by `id`,
    highest first. Ids are unique, so no two orders have the same key, and
    a key says exactly where an order sits in the listing (cursor.py relies
    on that).
    """
    return (-to_micros(order["created_at"]), -order["id"])


class OrderStore:
    def __init__(self):
        self._orders = {}

    def add(self, order):
        """Store a copy of `order` (a dict with an int `id`, a `created_at`
        string as described in timeutil.py, and any other fields)."""
        if not isinstance(order.get("id"), int):
            raise TypeError("order id must be an int")
        if order["id"] in self._orders:
            raise KeyError(f"duplicate order id {order['id']}")
        to_micros(order["created_at"])  # validate
        self._orders[order["id"]] = dict(order)

    def remove(self, order_id):
        del self._orders[order_id]

    def listing(self):
        """Copies of every order, in listing order (see `sort_key`)."""
        return [dict(o) for o in sorted(self._orders.values(), key=sort_key)]
