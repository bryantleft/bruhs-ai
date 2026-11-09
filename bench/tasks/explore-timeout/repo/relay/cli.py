"""`python3 -m relay.cli --config relay.ini` prints the effective settings."""

import argparse

from relay.conf import Settings
from relay.conf.defaults import DEFAULTS


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument("--config")
    args = parser.parse_args(argv)
    settings = Settings.load(args.config) if args.config else Settings()
    for key in sorted(DEFAULTS):
        print(f"{key} = {settings.get(key)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
