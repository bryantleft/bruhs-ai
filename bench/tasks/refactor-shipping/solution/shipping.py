"""Shipping quotes. Prices are in cents."""

from collections import namedtuple

Rate = namedtuple("Rate", "name base per_kg express_mult")

RATES = {
    "US": Rate("United States", 500, 120, 1.5),
    "CA": Rate("Canada", 650, 140, 1.6),
    "MX": Rate("Mexico", 700, 150, 1.8),
    "DE": Rate("Germany", 900, 210, 1.4),
    "FR": Rate("France", 920, 205, 1.4),
    "JP": Rate("Japan", 1200, 260, 1.3),
    "AU": Rate("Australia", 1300, 280, 1.7),
}


def rate_for(country):
    """The rate entry for `country`; `KeyError` when unknown."""
    return RATES[country]


def quote(country, weight_kg, express=False):
    """Price in cents to ship `weight_kg` to `country`."""
    if weight_kg <= 0:
        raise ValueError("weight must be positive")
    rate = rate_for(country)
    price = rate.base + rate.per_kg * weight_kg
    if express:
        price = price * rate.express_mult
    return int(round(price))


def invoice_line(country, weight_kg, express=False):
    """A one-line description for the invoice."""
    name = rate_for(country).name
    cents = quote(country, weight_kg, express)
    kind = "express" if express else "standard"
    return f"{name} {kind} {weight_kg:g}kg: ${cents // 100}.{cents % 100:02d}"


def cheapest_country(countries, weight_kg):
    """The country in `countries` with the lowest standard quote; ties keep
    the first one listed."""
    best = None
    best_price = None
    for country in countries:
        price = quote(country, weight_kg)
        if best_price is None or price < best_price:
            best, best_price = country, price
    return best
