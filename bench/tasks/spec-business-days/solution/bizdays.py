"""Business-day arithmetic. See SPEC.md for the exact rules."""

import calendar
import datetime

CONVENTIONS = ("following", "preceding", "modified_following", "modified_preceding")
ONE_DAY = datetime.timedelta(days=1)


def is_business_day(d, holidays=frozenset()):
    return d.weekday() < 5 and d not in holidays


def _step(d, direction, holidays):
    d += direction
    while not is_business_day(d, holidays):
        d += direction
    return d


def roll(d, convention, holidays=frozenset()):
    if convention not in CONVENTIONS:
        raise ValueError(f"unknown convention {convention!r}")
    if is_business_day(d, holidays):
        return d
    following = _step(d, ONE_DAY, holidays)
    preceding = _step(d, -ONE_DAY, holidays)
    if convention == "following":
        return following
    if convention == "preceding":
        return preceding
    if convention == "modified_following":
        return following if following.month == d.month else preceding
    return preceding if preceding.month == d.month else following


def add_business_days(d, n, holidays=frozenset()):
    if n == 0:
        return roll(d, "following", holidays)
    direction = ONE_DAY if n > 0 else -ONE_DAY
    for _ in range(abs(n)):
        d = _step(d, direction, holidays)
    return d


def add_months(d, months, end_of_month=False):
    index = d.year * 12 + (d.month - 1) + months
    year, month = divmod(index, 12)
    month += 1
    last = calendar.monthrange(year, month)[1]
    if end_of_month and d.day == calendar.monthrange(d.year, d.month)[1]:
        return datetime.date(year, month, last)
    return datetime.date(year, month, min(d.day, last))


def schedule(start, every_months, count, convention="modified_following", holidays=frozenset(), end_of_month=False):
    if every_months < 1 or count < 0:
        raise ValueError("every_months must be >= 1 and count >= 0")
    return [
        roll(add_months(start, k * every_months, end_of_month), convention, holidays)
        for k in range(1, count + 1)
    ]
