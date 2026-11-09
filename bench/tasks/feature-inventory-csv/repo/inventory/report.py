"""Human-readable stock reports."""


def text_report(items):
    """One aligned line per item, sorted by name."""
    lines = []
    for item in sorted(items, key=lambda item: item.name.lower()):
        lines.append(f"{item.sku:<8} {item.name:<24} {item.quantity:>5}")
    return "\n".join(lines) + ("\n" if lines else "")


def low_stock(items, threshold=5):
    """Items whose quantity is below `threshold`."""
    return [item for item in items if item.quantity < threshold]
