"""Inventory records."""

import json
from dataclasses import dataclass
from decimal import ROUND_HALF_UP, Decimal

CENT = Decimal("0.01")


def money(value):
    """`value` rounded half up to cents."""
    return Decimal(value).quantize(CENT, rounding=ROUND_HALF_UP)


@dataclass(frozen=True)
class Item:
    sku: str
    name: str
    quantity: int
    unit_price: Decimal

    @property
    def total_value(self):
        return money(self.quantity * self.unit_price)

    @classmethod
    def from_dict(cls, raw):
        return cls(
            sku=str(raw["sku"]),
            name=str(raw["name"]),
            quantity=int(raw["quantity"]),
            unit_price=Decimal(str(raw["unit_price"])),
        )


def load_items(path):
    """Items from a JSON file holding a list of objects."""
    with open(path, encoding="utf-8") as handle:
        return [Item.from_dict(raw) for raw in json.load(handle)]
