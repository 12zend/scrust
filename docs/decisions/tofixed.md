# `toFixed`

`sin`/`cos`/`tan` are `parseFloat(x.toFixed(10))`. `toFixed` rounds the *exact* binary value, ties to the larger magnitude. The fast path multiplies the 53-bit mantissa by `10^10` in `u128` and rounds half up; larger magnitudes use the exact decimal expansion (`{:.1100}`). `(-0).toFixed(10)` is `"0.0000000000"` (+0) but `(-1e-20).toFixed(10)` is `"-0.0000000000"`, which parses to -0.

`say` rounds non-integer numbers with `toFixed(2)` (`js_to_fixed_string`).
