# Invoice totals — specification v3

`invoice.compute_invoice(invoice)` turns an invoice draft into the amounts
printed on the invoice and reported to the tax office. Accounting reconciles
every printed amount to the cent, so this document is exact: where it says
how to round, that is the only correct rounding, and two implementations
that follow it give identical output for every input.

This document is the whole contract. Sections are referenced by number.

## 1. Input

`invoice` is a `dict`:

| key | type | required | meaning |
|---|---|---|---|
| `currency` | `str` | yes | ISO 4217 code, see §2 |
| `prices_include_tax` | `bool` | no (default `False`) | see §7 |
| `lines` | `list` of `dict` | yes | may be empty |
| `invoice_discount` | `str` | no (default `"0"`) | an amount in the currency, see §6 |

Each line is a `dict`:

| key | type | required | meaning |
|---|---|---|---|
| `quantity` | `str` | yes | a decimal number; may be negative (§8) or zero |
| `unit_price` | `str` | yes | a decimal number, `>= 0`, any number of decimals |
| `tax_rate` | `str` | yes | a decimal fraction, `0 <= rate < 1` (`"0.19"` is 19 %) |
| `discount_percent` | `str` | no (default `"0"`) | a decimal number, `0 <= p <= 100` |

Lines may carry other keys (`"description"`, `"sku"`, …); they are ignored.

Every value must have the type the tables give it; a value of any other
type raises `TypeError`. In particular every number in the input is a
string in plain decimal notation (`"12"`, `"-1.5"`, `"0.075"`, `"19.9900"`):
a number given as an `int`, a `float` or a `Decimal` raises `TypeError`, and
a string that is not a decimal number raises `ValueError`. (The keys that
are ignored can hold anything.) A value outside its allowed range
raises `ValueError`. The function never mutates its argument.

## 2. Currencies and the minor unit

Every amount is held and rounded in the currency's **minor unit**:

| currency | decimals |
|---|---|
| `EUR`, `USD`, `GBP`, `CHF` | 2 |
| `JPY`, `KRW` | 0 |
| `KWD`, `BHD` | 3 |

Any other code raises `ValueError`.

"Rounding an amount" always means rounding it to the currency's minor unit
(two decimals for EUR, none for JPY, three for KWD).

## 3. Rounding

Two rounding rules are used, and each step below says which one applies:

- **half away from zero** — a value exactly half-way between two minor
  units goes to the one farther from zero: `2.345 → 2.35`,
  `-2.345 → -2.35`, `0.125 → 0.13`.
- **half to even** — a value exactly half-way goes to the one whose last
  digit is even: `2.345 → 2.34`, `2.355 → 2.36`, `-0.125 → -0.12`.

A value that is not exactly half-way rounds to the nearest minor unit under
both rules. Intermediate results (products, quotients, shares) are exact
until they are rounded; binary floating point is not exact and must not be
used anywhere (`float("1.005") * 100` is `100.49999999999999`).

## 4. Line amounts

For each line, in order:

1. `amount = quantity × unit_price`, rounded **half away from zero**.
2. `line_discount = amount × discount_percent / 100`, computed from the
   rounded `amount` of step 1 and rounded **half away from zero**.
3. `net = amount − line_discount` (already a whole number of minor units).

## 5. Tax rates and groups

Lines are grouped by tax rate. Two rates are the same rate when they are
numerically equal: `"0.2"`, `"0.20"` and `"0.200"` are one group.

Tax is computed **per group, not per line**: the group's base is the sum of
its lines' `taxable` amounts (§6), and the group's tax is computed once
from that sum (§7). Summing per-line taxes gives a different, wrong result.

A group whose rate is zero is still a group: it is listed in the output
with a tax of zero.

## 6. Invoice discount

`invoice_discount` is a non-negative amount in the currency (major units,
e.g. `"5.00"` EUR). It must be a whole number of minor units: `"5.005"` EUR
or `"100.5"` JPY raises `ValueError`.

The invoice discount is spread over the lines before tax, so that each tax
group is taxed on what the customer actually pays:

1. The **eligible** lines are those whose `net` (§4) is greater than zero.
   Lines with a zero or negative `net` (§8) get an allocation of zero.
2. Let `D` be the invoice discount and `S` the sum of the eligible lines'
   `net`, both in minor units. If `D > S`, raise `ValueError`. (When there
   are no eligible lines, `S` is zero, so any `D > 0` raises.)
3. Each eligible line's exact share is `D × net / S` minor units. Each line
   first gets the whole part of its share (rounded down).
4. The minor units still left over (`D` minus the sum of the whole parts)
   are handed out one each to the eligible lines with the largest
   fractional parts of their share. When two lines have the same fractional
   part, the one that comes first in `lines` gets its unit first.
5. `allocated` is what each line received, and
   `taxable = net − allocated`.

The allocations always add up to exactly `D`.

## 7. Tax

For each group, `base` is the sum of `taxable` over the group's lines.

**Prices without tax** (`prices_include_tax` false, the default): the base
is a net amount.

- `tax = base × rate`, rounded **half to even**.
- the group's net is `base`; its gross is `base + tax`.

**Prices including tax** (`prices_include_tax` true): every `unit_price` is
a gross price, so `amount`, `net`, `taxable` and `base` are gross amounts.
The tax contained in them is taken out per group:

- `tax = base × rate / (1 + rate)`, rounded **half to even**.
- the group's gross is `base`; its net is `base − tax`.

Line amounts (§4) and the invoice discount (§6) are computed the same way
in both modes; only this section differs.

### 7.1 Tax per line

Each printed line also shows its share of its group's tax (`tax` in §9).
The line taxes of a group always add up to exactly the group's tax:

1. A line's **exact tax**, in minor units and not rounded, is
   `taxable × rate` (prices without tax) or `taxable × rate / (1 + rate)`
   (prices including tax).
2. Each line of the group first gets its exact tax **rounded down, toward
   negative infinity**: `2.7 → 2`, `-2.3 → -3`, `-4 → -4` (minor units).
3. The group's tax (§7) minus the sum of these is never negative. It is
   handed out one minor unit each to the group's lines with the largest
   fractional parts, where a line's fractional part is its exact tax minus
   its rounded-down value (`2.7` has `0.7`, `-2.3` also has `0.7`). When two
   lines have the same fractional part, the one that comes first in
   `lines` gets its unit first.

## 8. Returns and credits

A negative `quantity` is a returned item: its `amount`, `line_discount`,
`net` and `taxable` are negative (`line_discount` is negative or zero, so a
discounted return refunds less). Round them as §4 says; the rounding rules
are symmetric around zero. A return never receives part of the invoice
discount (§6 step 1), but it does belong to its tax group, so it lowers that
group's base, possibly below zero; a negative base gives a negative (or
zero) tax under the same rounding rule.

## 9. Output

A new `dict`:

```python
{
    "currency": "EUR",
    "lines": [  # one per input line, in input order
        {"amount": "...", "line_discount": "...", "allocated": "...",
         "taxable": "...", "tax": "..."},
    ],
    "taxes": [  # one per group, highest rate first
        {"rate": "0.19", "base": "...", "tax": "..."},
    ],
    "net_total": "...",  # sum of the groups' net
    "tax_total": "...",  # sum of the groups' tax
    "total": "...",      # net_total + tax_total (= sum of the groups' gross)
}
```

Amounts are strings with exactly the currency's number of decimals and a
`.` separator, no thousands separators, a leading `-` for negative values:
`"1234.50"`, `"-0.07"`, `"0.00"` (EUR); `"1234"`, `"-7"` (JPY); `"1.250"`
(KWD). Zero is always written without a sign (`"0.00"`, never `"-0.00"`).

`rate` is the group's rate written with no trailing zeros after the decimal
point, and without the point if nothing is left after it: `"0.190"` and
`"0.19"` both print `"0.19"`, `"0.00"` prints `"0"`, `"0.075"` stays
`"0.075"`. Groups are ordered by rate, highest first.

An invoice with no lines has empty `lines` and `taxes` and zero totals.

## 10. Worked example

```python
compute_invoice({
    "currency": "EUR",
    "lines": [
        {"quantity": "3", "unit_price": "3.335", "tax_rate": "0.19"},
        {"quantity": "1", "unit_price": "10.00", "tax_rate": "0.07",
         "discount_percent": "15"},
        {"quantity": "-1", "unit_price": "2.50", "tax_rate": "0.19"},
    ],
    "invoice_discount": "1.00",
})
```

- Line 1: `3 × 3.335 = 10.005 → 10.01` (half away from zero); no discount;
  `net = 10.01`.
- Line 2: `amount = 10.00`, `line_discount = 1.50`, `net = 8.50`.
- Line 3: a return, `amount = -2.50`, `net = -2.50`, not eligible.
- Invoice discount: `D = 100` cents, `S = 1001 + 850 = 1851`. Shares
  `100 × 1001 / 1851 = 54.079…` and `100 × 850 / 1851 = 45.920…`; whole parts
  54 + 45 = 99, the leftover cent goes to line 2 (`.920 > .079`):
  allocations `0.54`, `0.46`, `0.00`; taxable `9.47`, `8.04`, `-2.50`.
- Group 0.19: `base = 9.47 − 2.50 = 6.97`, `tax = 1.3243 → 1.32`.
  Line taxes (cents): exact `947 × 0.19 = 179.93` and
  `-250 × 0.19 = -47.5`, rounded down `179` and `-48`, sum `131`; the one
  cent left goes to line 1 (fraction `.93 > .5`): `1.80` and `-0.48`.
- Group 0.07: `base = 8.04`, `tax = 0.5628 → 0.56`; line 2's tax is
  `56.28 → 56`, nothing left over: `0.56`.
- `net_total = 15.01`, `tax_total = 1.88`, `total = 16.89`.

## 11. Changes from v2

- v2 rounded the group tax half away from zero; v3 rounds it half to even
  (§7). Line amounts and discounts are still half away from zero (§4).
- v2 let returns absorb part of the invoice discount; v3 does not (§6).
- v2 computed tax per line. v3 computes it per group (§5).
