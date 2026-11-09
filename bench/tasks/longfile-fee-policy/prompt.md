Finance reports that the late fees we charge some accounts do not match the
billing policy. `POLICY.md` is the source of truth (read all of it, including
amendments); `fees.py` implements `late_fee(balance, status)`.

Make `late_fee` follow `POLICY.md` exactly for every status. Keep its name,
signature and `Decimal` return type. Existing tests (`python3 -m unittest`)
must keep passing.
