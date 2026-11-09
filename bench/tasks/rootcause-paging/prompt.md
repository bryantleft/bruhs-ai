A bug was filed against the back-office order listing, `list_orders` in
`orders/api.py`: when several orders are placed in the same second, some of
them never show up while the user pages through the list. `tests/test_api.py`
has a failing test that reproduces it.

The behaviour `list_orders` promises is in its docstring, and the other
modules in `orders/` document their parts the same way; the docstrings are
the spec. Find the actual cause, and fix every place in the package where
the code does not do what its docstring says, not only the reported symptom.
Keep the public names and signatures, Python standard library only. Run the
tests with `python3 -m unittest` when you are done.
