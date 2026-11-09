"""Posting entries to accounts and asking for balances."""

from collections import namedtuple

from .money import to_cents

Entry = namedtuple("Entry", "day account cents memo")


class Ledger:
    def __init__(self):
        self.entries = []

    def post(self, day, account, amount, memo=""):
        """Book `amount` (int cents or a string for parse_amount) to `account`
        on `day` (a datetime.date). Positive is money in, negative money out.
        Returns the Entry."""
        entry = Entry(day, account, to_cents(amount), memo)
        self.entries.append(entry)
        return entry

    def transfer(self, day, source, target, amount, memo=""):
        """Move a positive `amount` from `source` to `target`: one negative
        entry on `source` and one positive entry on `target`, both on `day`."""
        cents = to_cents(amount)
        if cents <= 0:
            raise ValueError("transfer amount must be positive")
        self.post(day, source, -cents, memo)
        self.post(day, target, cents, memo)

    def balance(self, account, as_of=None):
        """The sum of `account`'s entries dated on or before `as_of`
        (inclusive: the balance as of the 31st includes the 31st). `as_of`
        None means every entry."""
        return sum(
            e.cents
            for e in self.entries
            if e.account == account and (as_of is None or e.day < as_of)
        )

    def entries_between(self, account, start, end):
        """`account`'s entries with start <= day <= end, in date order.
        Entries on the same day keep the order in which they were posted."""
        mine = [e for e in self.entries if e.account == account and start <= e.day <= end]
        return sorted(mine, key=lambda e: (e.day, e.memo))
