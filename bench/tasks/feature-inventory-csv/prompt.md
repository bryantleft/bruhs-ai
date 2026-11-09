Add CSV export to the inventory package.

1. In `inventory/models.py`, give `Item` a read-only `total_value` property:
   `quantity * unit_price` as a `Decimal` rounded to 2 decimal places
   (round half up).
2. In `inventory/report.py`, add `csv_report(items) -> str` that returns CSV
   text with the header row `sku,name,quantity,unit_price,total_value`, then
   one row per item sorted by `sku`. Money columns use exactly two decimals, rounded half up
   (`3.50`). Use the `csv` module so names containing commas or quotes are
   quoted correctly. Lines end with `\n`.
3. Add `--csv` to `inventory/cli.py` so `python3 -m inventory.cli --csv
   stock.json` prints `csv_report` instead of the text report.

Existing tests (`python3 -m unittest`) must keep passing.
