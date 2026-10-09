# Where string vs number is observable (§8.6)

A canonical numeric string (`String(Number(s)) === s`, digits with an optional fraction, never `-0`) may be stored as `f64` when it cannot reach a sink that distinguishes it from the number. The lattice splits `STR_CANON` into integer-like and fractional canonical strings because some sinks only tell fractional ones apart.

| Sink | Distinguishes | Handling |
|---|---|---|
| `data_listcontents` | `"1"` vs `1` (single-character test) | list (and everything flowing into it) keeps kinds |
| `switch costume/backdrop` | only if a costume/backdrop is named like a canonical number | sensitive only then |
| `play sound` | only if a sound is named like a canonical number (`sounds[i].name === value` is strict) | sensitive only then |
| `say`/`think` | numbers are rounded to 2 decimals: `0.1234` vs `"0.1234"` | sensitive for fractional canonical strings |
| variable/list monitors | numeric rounding in the monitor | same as `say` |
| `key pressed?` | numbers 32, 37-40, 48-90 are key codes | sensitive |
| dynamic `(x) of (sprite)` | raw read | all variables with that name are sensitive |
| unsupported blocks | unknown | sensitive |

Not sinks: `pick random` (`isInt` agrees for canonical strings), list `contains` (identity is subsumed by `compare`), list indices, colour inputs, `toString`-first inputs.

Sensitivity is propagated backwards along raw value flows (`set v to (w)`, `add (v) to list`, procedure arguments, `item of list`). `scrust check --types` prints the result. Soundness is tested by running every conformance case both with `--no-unbox` (exact typed comparison) and unboxed (canonical strings and their numbers compare equal).
