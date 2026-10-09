# Math functions: `libm` crate, with two pinned cases

**Decision.** `sin/cos/tan/asin/acos/atan/exp/ln/pow` use the pure-Rust `libm` crate (fdlibm lineage) on every platform, so results do not depend on the host libm. Two exceptions make common results exact:

- `10 ^ n` for integer `|n| <= 22` is computed from an exact table (`10^n` or `1/10^-n`), which is what any correctly rounded `pow` returns.
- `e ^ 1` returns `Math.E` (the `libm` port is 1 ULP high there).

**Measured (Node 26.9 / V8 14.6, macOS arm64, 50,000 random inputs per function).** Share of inputs where `libm` differs from V8 by 1 ULP: acos 0.6 %, asin 1.3 %, atan 0.02 %, exp 0.14 %, log 0.3 %, sin/cos/tan about 1.1 % (invisible after Scratch's `toFixed(10)` except on rounding boundaries), `10 ^ x` for non-integer `x` 7 %. The host libm (Apple) was further from V8 for everything except `pow`.

**Why not chase V8 exactly.** V8 itself is not stable here: 14.6 uses fdlibm-style code for most functions and glibc-derived `sin/cos`; current V8 `main` routes all of them to LLVM libc (correctly rounded). A Scratch project therefore already sees 1-ULP differences between browser versions. `log 1000 = 2.9999999999999996` (i.e. `ln(x)/LN10`, not `log10`) is reproduced exactly.

**Consequence for tests.** The fuzzer only uses `abs, floor, ceiling, sqrt, sin, cos, tan`; the spec table uses exactly representable cases for the others.
