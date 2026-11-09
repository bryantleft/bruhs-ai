"""Invoice totals. See SPEC.md for the contract."""

import math
import re
from decimal import ROUND_HALF_EVEN, ROUND_HALF_UP, Decimal
from fractions import Fraction

MINOR_UNITS = {
    "EUR": 2,
    "USD": 2,
    "GBP": 2,
    "CHF": 2,
    "JPY": 0,
    "KRW": 0,
    "KWD": 3,
    "BHD": 3,
}

_NUMBER = re.compile(r"-?\d+(\.\d+)?")


def _num(value, what):
    if not isinstance(value, str):
        raise TypeError(f"{what} must be a string, got {type(value).__name__}")
    text = value.strip()
    if not _NUMBER.fullmatch(text):
        raise ValueError(f"{what} is not a decimal number: {value!r}")
    return Decimal(text)


def _round(value, decimals, mode):
    """Round to `decimals` places and return whole minor units (int)."""
    scaled = (value * (Decimal(10) ** decimals)).quantize(Decimal(1), rounding=mode)
    return int(scaled)


def _fmt(units, decimals):
    sign = "-" if units < 0 else ""
    units = abs(units)
    if decimals == 0:
        return f"{sign}{units}"
    whole, frac = divmod(units, 10**decimals)
    return f"{sign}{whole}.{frac:0{decimals}d}"


def _fmt_rate(rate):
    text = format(rate, "f")
    if "." in text:
        text = text.rstrip("0").rstrip(".")
    return text or "0"


def _typed(value, kind, what):
    if not isinstance(value, kind) or (kind is not bool and isinstance(value, bool)):
        raise TypeError(f"{what} must be a {kind.__name__}, got {type(value).__name__}")
    return value


def compute_invoice(invoice):
    _typed(invoice, dict, "invoice")
    currency = _typed(invoice["currency"], str, "currency")
    if currency not in MINOR_UNITS:
        raise ValueError(f"unknown currency {currency!r}")
    dec = MINOR_UNITS[currency]
    scale = Decimal(10) ** dec
    inclusive = _typed(invoice.get("prices_include_tax", False), bool, "prices_include_tax")
    _typed(invoice["lines"], list, "lines")
    for line in invoice["lines"]:
        _typed(line, dict, "a line")

    rows = []
    for line in invoice["lines"]:
        qty = _num(line["quantity"], "quantity")
        price = _num(line["unit_price"], "unit_price")
        rate = _num(line["tax_rate"], "tax_rate")
        pct = _num(line.get("discount_percent", "0"), "discount_percent")
        if price < 0:
            raise ValueError("unit_price must be >= 0")
        if not (0 <= rate < 1):
            raise ValueError("tax_rate must be in [0, 1)")
        if not (0 <= pct <= 100):
            raise ValueError("discount_percent must be in [0, 100]")
        amount = _round(qty * price, dec, ROUND_HALF_UP)
        line_discount = _round(Decimal(amount) / scale * pct / 100, dec, ROUND_HALF_UP)
        rows.append({"rate": rate, "amount": amount, "line_discount": line_discount,
                     "net": amount - line_discount})

    d_value = _num(invoice.get("invoice_discount", "0"), "invoice_discount")
    if d_value < 0:
        raise ValueError("invoice_discount must be >= 0")
    d_scaled = d_value * scale
    if d_scaled != d_scaled.to_integral_value():
        raise ValueError("invoice_discount is not a whole number of minor units")
    total_d = int(d_scaled)

    eligible = [i for i, row in enumerate(rows) if row["net"] > 0]
    s = sum(rows[i]["net"] for i in eligible)
    if total_d > s:
        raise ValueError("invoice_discount exceeds the eligible lines")
    for row in rows:
        row["allocated"] = 0
    if total_d:
        rems = []
        for i in eligible:
            whole, rem = divmod(total_d * rows[i]["net"], s)
            rows[i]["allocated"] = whole
            rems.append((-rem, i))
        left = total_d - sum(rows[i]["allocated"] for i in eligible)
        for _, i in sorted(rems)[:left]:
            rows[i]["allocated"] += 1
    for row in rows:
        row["taxable"] = row["net"] - row["allocated"]

    groups = {}
    members = {}
    for i, row in enumerate(rows):
        groups.setdefault(row["rate"], 0)
        groups[row["rate"]] += row["taxable"]
        members.setdefault(row["rate"], []).append(i)

    taxes = []
    net_total = tax_total = 0
    for rate in sorted(groups, reverse=True):
        base = groups[rate]
        exact = Decimal(base) / scale
        if inclusive:
            tax = _round(exact * rate / (1 + rate), dec, ROUND_HALF_EVEN)
            net = base - tax
        else:
            tax = _round(exact * rate, dec, ROUND_HALF_EVEN)
            net = base
        net_total += net
        tax_total += tax
        # §7.1: split the group tax over its lines (exact, in minor units).
        r = Fraction(rate)
        factor = r / (1 + r) if inclusive else r
        line_exact = {i: rows[i]["taxable"] * factor for i in members[rate]}
        for i in members[rate]:
            rows[i]["tax"] = math.floor(line_exact[i])
        left = tax - sum(rows[i]["tax"] for i in members[rate])
        assert left >= 0
        order = sorted(members[rate], key=lambda i: (-(line_exact[i] - rows[i]["tax"]), i))
        for i in order[:left]:
            rows[i]["tax"] += 1
        taxes.append({"rate": _fmt_rate(rate), "base": _fmt(base, dec), "tax": _fmt(tax, dec)})

    return {
        "currency": currency,
        "lines": [
            {
                "amount": _fmt(row["amount"], dec),
                "line_discount": _fmt(row["line_discount"], dec),
                "allocated": _fmt(row["allocated"], dec),
                "taxable": _fmt(row["taxable"], dec),
                "tax": _fmt(row["tax"], dec),
            }
            for row in rows
        ],
        "taxes": taxes,
        "net_total": _fmt(net_total, dec),
        "tax_total": _fmt(tax_total, dec),
        "total": _fmt(net_total + tax_total, dec),
    }
