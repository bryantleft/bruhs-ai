"""Human-readable stock reports."""

import csv
import io

from inventory.models import money


def text_report(items):
    """One aligned line per item, sorted by name."""
    lines = []
    for item in sorted(items, key=lambda item: item.name.lower()):
        lines.append(f"{item.sku:<8} {item.name:<24} {item.quantity:>5}")
    return "\n".join(lines) + ("\n" if lines else "")


def low_stock(items, threshold=5):
    """Items whose quantity is below `threshold`."""
    return [item for item in items if item.quantity < threshold]


def csv_report(items):
    """CSV text, one row per item sorted by sku."""
    out = io.StringIO()
    writer = csv.writer(out, lineterminator="\n")
    writer.writerow(["sku", "name", "quantity", "unit_price", "total_value"])
    for item in sorted(items, key=lambda item: item.sku):
        writer.writerow(
            [item.sku, item.name, item.quantity, money(item.unit_price), item.total_value]
        )
    return out.getvalue()
