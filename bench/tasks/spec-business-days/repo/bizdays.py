"""Business-day arithmetic. See SPEC.md for the exact rules."""

import datetime  # noqa: F401  (used by the implementation)

CONVENTIONS = ("following", "preceding", "modified_following", "modified_preceding")


def is_business_day(d, holidays=frozenset()):
    raise NotImplementedError


def roll(d, convention, holidays=frozenset()):
    raise NotImplementedError


def add_business_days(d, n, holidays=frozenset()):
    raise NotImplementedError


def add_months(d, months, end_of_month=False):
    raise NotImplementedError


def schedule(start, every_months, count, convention="modified_following", holidays=frozenset(), end_of_month=False):
    raise NotImplementedError
