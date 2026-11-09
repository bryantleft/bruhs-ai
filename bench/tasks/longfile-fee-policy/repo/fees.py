"""Late fees. See POLICY.md §14."""

from decimal import ROUND_HALF_UP, Decimal

CENT = Decimal("0.01")
RATE = Decimal("0.05")
MINIMUM = Decimal("2.00")
MAXIMUM = Decimal("40.00")
WRITE_OFF_BELOW = Decimal("1.00")
STATUSES = ("active", "grace", "suspended", "closed")


def base_fee(balance):
    """5% of the overdue balance, clamped to the policy minimum and maximum."""
    fee = (Decimal(balance) * RATE).quantize(CENT, rounding=ROUND_HALF_UP)
    return min(MAXIMUM, max(MINIMUM, fee))


def late_fee(balance, status):
    """The late fee for an overdue `balance` on an account in `status`."""
    if status not in STATUSES:
        raise ValueError(f"unknown status {status!r}")
    balance = Decimal(balance)
    if balance < WRITE_OFF_BELOW or status == "closed":
        return Decimal("0.00")
    return base_fee(balance)
