# Procedure resolution

- Parameters come from the first `procedures_prototype` with the proccode in block order; the body from the first `procedures_definition` (they can differ when a project has duplicate definitions).
- Arguments are stored by *name*: with duplicate names the last one wins.
- An argument reporter outside a definition, or naming a parameter the enclosing definition does not have, reports `0` (both reporter kinds).
- A call without prototype or without definition evaluates its inputs and does nothing.
- Warp mode is inherited by callees. Each procedure is compiled in up to two variants (warp / normal); a variant that cannot suspend becomes a plain `fn(&mut World, ...)`, otherwise a boxed `async` function.
