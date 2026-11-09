"""`python3 -m inventory.cli stock.json` prints a stock report."""

import argparse
import sys

from inventory.models import load_items
from inventory.report import csv_report, text_report


def main(argv=None):
    parser = argparse.ArgumentParser(description="Print a stock report.")
    parser.add_argument("path", help="JSON file with the stock list")
    parser.add_argument("--csv", action="store_true", help="print CSV instead")
    args = parser.parse_args(argv)
    items = load_items(args.path)
    sys.stdout.write(csv_report(items) if args.csv else text_report(items))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
