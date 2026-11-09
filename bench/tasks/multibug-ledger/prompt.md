Customers say their account statements from the `ledger` package are wrong.
Two symptoms have been reported so far:

- an amount typed as `1.5` is booked as $1.05;
- a refund entered as `-0.50` shows up as a 50-cent charge.

`tests/test_money.py` has a failing test for the first one; nothing covers the
second yet. We think these are not the only bugs: review `ledger/money.py`,
`ledger/accounts.py` and `ledger/report.py` against their docstrings (the
docstrings are the spec) and fix every place where the code does not do what
its docstring says. Keep the public names and signatures. Run the tests with
`python3 -m unittest` when you are done.
