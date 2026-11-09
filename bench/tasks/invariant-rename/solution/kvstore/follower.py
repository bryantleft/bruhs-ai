"""A replica that follows the store's change log."""

from .ops import apply_entry


class Follower:
    """Applies log entries shipped from the primary.

    Entries arrive one at a time and the follower acknowledges by count, so
    each entry is one transaction on the replica. That is why a public Store
    method must write exactly one log entry per call (one _commit): a method
    that commits twice for one logical change is shipped as two
    transactions, and a replica whose link drops between them is left
    half-way (this caused the 2025-08 settings outage). Several changes that
    belong together go in one "batch" entry. The replica understands only
    the ops in ops.py.
    """

    def __init__(self):
        self.data = {}
        self.applied = 0

    def apply(self, entry):
        apply_entry(self.data, entry)
        self.applied += 1

    def catch_up(self, log):
        for entry in log[self.applied:]:
            self.apply(entry)
