"""Shipping quotes. Prices are in cents."""


def quote(country, weight_kg, express=False):
    """Price in cents to ship `weight_kg` to `country`."""
    if weight_kg <= 0:
        raise ValueError("weight must be positive")
    if country == "US":
        base, per_kg, express_mult = 500, 120, 1.5
    elif country == "CA":
        base, per_kg, express_mult = 650, 140, 1.6
    elif country == "MX":
        base, per_kg, express_mult = 700, 150, 1.8
    elif country == "DE":
        base, per_kg, express_mult = 900, 210, 1.4
    elif country == "FR":
        base, per_kg, express_mult = 920, 205, 1.4
    elif country == "JP":
        base, per_kg, express_mult = 1200, 260, 1.3
    elif country == "AU":
        base, per_kg, express_mult = 1300, 280, 1.7
    else:
        raise KeyError(country)
    price = base + per_kg * weight_kg
    if express:
        price = price * express_mult
    return int(round(price))


def invoice_line(country, weight_kg, express=False):
    """A one-line description for the invoice."""
    if country == "US":
        name = "United States"
    elif country == "CA":
        name = "Canada"
    elif country == "MX":
        name = "Mexico"
    elif country == "DE":
        name = "Germany"
    elif country == "FR":
        name = "France"
    elif country == "JP":
        name = "Japan"
    elif country == "AU":
        name = "Australia"
    else:
        raise KeyError(country)
    cents = quote(country, weight_kg, express)
    kind = "express" if express else "standard"
    return f"{name} {kind} {weight_kg:g}kg: ${cents // 100}.{cents % 100:02d}"


def cheapest_country(countries, weight_kg):
    """The country in `countries` with the lowest standard quote; ties keep
    the first one listed."""
    best = None
    best_price = None
    for country in countries:
        if country == "US":
            price = quote("US", weight_kg)
        elif country == "CA":
            price = quote("CA", weight_kg)
        elif country == "MX":
            price = quote("MX", weight_kg)
        elif country == "DE":
            price = quote("DE", weight_kg)
        elif country == "FR":
            price = quote("FR", weight_kg)
        elif country == "JP":
            price = quote("JP", weight_kg)
        elif country == "AU":
            price = quote("AU", weight_kg)
        else:
            raise KeyError(country)
        if best_price is None or price < best_price:
            best, best_price = country, price
    return best
