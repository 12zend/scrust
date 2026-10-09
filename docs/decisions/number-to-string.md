# Number -> string: shortest digits plus an explicit tie rule

`String(number)` is built from Rust's shortest round-trip digits (`{:e}`), laid out with the ECMAScript `Number::toString` rules (fixed notation for `1e-7 <= |x| < 1e21`, `e+21` style otherwise, `-0 -> "0"`).

Shortest-digit generators may pick either candidate when two shortest decimals are equally close to the value; ECMAScript requires the one with an even last digit. Example: `0.95453643798828125` must print as `0.9545364379882812`, Rust gives `...813`. `fix_tie_to_even` detects exact ties (the value is an odd multiple of `2^-j` with exactly one more decimal digit than the shortest form) and re-picks if the even candidate also round-trips.

Verified against Node with 5,000,000 random numbers (0 mismatches); `Number(string)` likewise with 5,000,000 random strings (`conformance/value/gen.js`).
