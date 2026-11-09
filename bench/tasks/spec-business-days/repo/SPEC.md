# bizdays

Business-day arithmetic for a payments calendar. Every function takes and
returns `datetime.date` values and never mutates its arguments. `holidays`
is any container of `datetime.date` supporting `in` (default: empty).

A **business day** is a Monday to Friday that is not in `holidays`.

## `is_business_day(d, holidays=frozenset()) -> bool`

True when `d` is a business day.

## `roll(d, convention, holidays=frozenset()) -> date`

Moves a date that is not a business day onto one. The convention is checked
first: any value other than the four below raises `ValueError`, even when `d`
is already a business day. A business day is returned unchanged.
Otherwise:

- `"following"`: the first business day after `d`.
- `"preceding"`: the last business day before `d`.
- `"modified_following"`: the `"following"` date, unless it is in a different
  calendar month from `d`; then the `"preceding"` date.
- `"modified_preceding"`: the `"preceding"` date, unless it is in a different
  calendar month from `d`; then the `"following"` date.

## `add_business_days(d, n, holidays=frozenset()) -> date`

- `n > 0`: step forward one calendar day at a time; the result is the day on
  which the `n`-th business day is reached. `d` itself is never counted,
  whether or not it is a business day: Friday + 1 is the next Monday, and
  Saturday + 1 is also that Monday.
- `n < 0`: the same, stepping backward: Monday − 1 is the previous Friday,
  Sunday − 1 is the Friday before it.
- `n == 0`: `roll(d, "following", holidays)`.

## `add_months(d, months, end_of_month=False) -> date`

The same day of the month `months` calendar months later (`months` may be
negative or zero). When the target month is shorter, clamp to its last day:
January 31 + 1 month is February 28 (February 29 in a leap year). Leap years
follow the Gregorian rule (divisible by 4, except centuries not divisible by
400).

With `end_of_month=True` and `d` the **last day of its month**, the result is
the last day of the target month instead: February 28, 2027 + 1 month is
March 31, 2027, and April 30 + 1 month is May 31. `end_of_month` has no effect
on any other day.

## `schedule(start, every_months, count, convention="modified_following", holidays=frozenset(), end_of_month=False) -> list[date]`

Payment dates. For `k = 1 .. count`, the unadjusted date is
`add_months(start, k * every_months, end_of_month)`. Each one is computed
**from `start`**, never from the previous payment, so a January 31 start with
monthly payments gives February 28, March 31, April 30, … (not February 28,
March 28, April 28, …). Each unadjusted date is then rolled with `roll(...,
convention, holidays)`. `start` itself is not a payment date.

`count == 0` returns `[]`. `every_months < 1` or `count < 0` raises
`ValueError`.
