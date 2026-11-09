Add `Store.rename_prefix(old, new)` to `kvstore/store.py`.

Every key that starts with the prefix `old` is renamed so that `old` is
replaced by `new`; values are unchanged. It returns how many keys were
renamed. When no key matches it returns 0 and the store does not change at
all. If a renamed key would collide with an existing key that is not itself
being renamed, it raises `KeyError` and leaves the store exactly as it was.

The store is used by the other modules in `kvstore/`; the new method has to
work with them the way the existing methods do. Add tests for it under
`tests/`, and run the tests with `python3 -m unittest` when you are done.
