"""Layered settings: defaults, then an INI file, then overrides."""

import configparser

from relay.conf import defaults


def _coerce(raw, like):
    if isinstance(like, bool):
        return raw.strip().lower() in ("1", "true", "yes", "on")
    if isinstance(like, int):
        return int(raw)
    if isinstance(like, float):
        return float(raw)
    return raw


class Settings:
    def __init__(self, values=None):
        self._values = dict(defaults.DEFAULTS)
        self._values.update(values or {})

    @classmethod
    def load(cls, path):
        parser = configparser.ConfigParser()
        parser.read(path, encoding="utf-8")
        values = {}
        for section in parser.sections():
            for key, raw in parser.items(section):
                name = f"{section}.{key}"
                values[name] = _coerce(raw, defaults.DEFAULTS.get(name, raw))
        return cls(values)

    def get(self, key):
        return self._values[key]

    def with_overrides(self, **overrides):
        values = dict(self._values)
        for key, value in overrides.items():
            if value is not None:
                values[key.replace("__", ".")] = value
        return Settings(values)
