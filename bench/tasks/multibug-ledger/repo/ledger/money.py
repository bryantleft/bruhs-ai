"""Money amounts. Every amount inside the ledger is an integer number of cents."""


def parse_amount(text):
    """Parse a decimal amount into integer cents.

    Accepts an optional sign, a whole part and up to two fraction digits:
    "12.34" -> 1234, "7" -> 700, "1.5" -> 150, ".25" -> 25, "+3.25" -> 325,
    "-0.50" -> -50, "-12.3" -> -1230. Surrounding whitespace is ignored and the
    whole part may use "," as a thousands separator ("1,234.56" -> 123456).

    Raises ValueError for an empty string, more than two fraction digits or
    anything that is not a number.
    """
    text = text.strip().replace(",", "")
    if not text:
        raise ValueError("empty amount")
    whole, _, frac = text.partition(".")
    if len(frac) > 2:
        raise ValueError(f"too many decimal places: {text!r}")
    if frac and not frac.isdigit():
        raise ValueError(f"not an amount: {text!r}")
    if whole in ("", "+", "-"):
        whole += "0"
    cents = int(whole) * 100
    if frac:
        value = int(frac)
        cents += value if cents >= 0 else -value
    return cents


def to_cents(amount):
    """An int is already cents; a str is parsed with parse_amount."""
    if isinstance(amount, bool):
        raise TypeError("amount must be int cents or str")
    if isinstance(amount, int):
        return amount
    if isinstance(amount, str):
        return parse_amount(amount)
    raise TypeError("amount must be int cents or str")
