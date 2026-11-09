"""`python3 -m inventory.cli stock.json` prints a stock report."""

import argparse
import sys

from inventory.models import load_items
from inventory.report import text_report


def main(argv=None):
    parser = argparse.ArgumentParser(description="Print a stock report.")
    parser.add_argument("path", help="JSON file with the stock list")
    args = parser.parse_args(argv)
    items = load_items(args.path)
    sys.stdout.write(text_report(items))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
