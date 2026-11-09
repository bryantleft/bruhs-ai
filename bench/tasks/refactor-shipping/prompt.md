Refactor `shipping.py`. The per-country rates are copy-pasted into
`quote()`, `invoice_line()` and `cheapest_country()` as long `if`/`elif`
chains.

- Move the rates into one module-level dict named `RATES` that maps each
  country code to its rate data, and add `rate_for(country)` that returns the
  entry for a country (raising `KeyError` for unknown countries, as the
  functions do today).
- Make every function use `RATES` / `rate_for` so each country code appears
  only once in the file.
- Behaviour must not change: same results, same rounding, same errors.
  The tests in `tests/` must stay green — do not edit them.
