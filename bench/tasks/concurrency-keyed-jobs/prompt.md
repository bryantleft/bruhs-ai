`jobs.KeyedSerializer` (in `jobs/serializer.py`) makes sure two transfers on
the same account never run at the same time. Production logs show that
sometimes they do: it happens around requests that time out while their
transfer is still waiting for its turn.

The tests in `tests/` pass. We think this is not the only bug: the class
docstring lists the rules callers rely on and is the spec. Review the whole
class against it and fix every place where the code does not do what the
docstring says. Keep the public names and signatures, Python standard
library only. Run the tests with `python3 -m unittest` when you are done.
